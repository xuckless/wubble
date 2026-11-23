//! The private line as one table: what each command that reaches from one side to another may
//! do ([`crossing`]). Every command of the model that could cross reads it; nothing else decides.
//! A pair the table does not list is refused (fails closed).

use crate::space::Side;

/// A command that reaches from something on one side (`src`) to something on another (`dst`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// A tab to another Space (`Command::MoveTab`): the tab's Space, the destination.
    MoveTab,
    /// A tab to an existing Main window (`Command::MoveTabToWindow`): the tab, the window.
    MoveToWindow,
    /// The focused window to a Space (`Command::SwitchSpace`): the focused window, the Space.
    SwitchSpace,
    /// A new tab in a Space with a container named for it (`place_new_tab`): the Space, the
    /// container.
    NewTab,
    /// A Space's default container (`Command::SetSpaceContainer`): the Space, the container.
    SetSpaceContainer,
    /// A tab reopened in a container (`Command::ReopenInContainer`): the tab, the container.
    ReopenInContainer,
    /// `Command::DeleteContainer`: the container itself (both sides).
    DeleteContainer,
    /// `Command::DeleteSpace`: the Space itself.
    DeleteSpace,
    /// `Command::MarkAgentSpace`: the Space itself.
    MarkAgentSpace,
    /// An agent session's tab (`Command::OpenInSpace`): the session (normal), the Space.
    OpenInSpace,
    /// An extension's tab (`Command::ExtensionTab`): the extension's container, the Space.
    ExtensionTab,
    /// A link from another app (`Command::OpenLittle`): the app (normal), the Space.
    OpenLittle,
    /// A Little window's page becoming a tab (`Command::PromoteLittle`): its tab, the Space.
    PromoteLittle,
}

/// What the command does across the line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Crossing {
    Allow,
    Refuse,
    /// Done in the last normal window instead: a private window is locked to its Space, so a
    /// normal Space it asks for is shown in a normal window (`Model::set_active_space`).
    ToNormalWindow,
}

use Crossing::{Allow, ToNormalWindow};
use Reach::*;
use Side::{Normal, Private};

/// Every pair that is not refused. A private window keeps to itself: its tabs move only among
/// private windows and its in-memory container is never renamed, reassigned, deleted, named
/// for a normal tab or reached by an agent, an extension or another app.
const CROSSING: &[(Reach, Side, Side, Crossing)] = &[
    (MoveTab, Normal, Normal, Allow),
    (MoveTab, Private, Private, Allow),
    (MoveToWindow, Normal, Normal, Allow),
    (MoveToWindow, Private, Private, Allow),
    (SwitchSpace, Normal, Normal, Allow),
    (SwitchSpace, Private, Normal, ToNormalWindow),
    (NewTab, Normal, Normal, Allow),
    (NewTab, Private, Private, Allow),
    (SetSpaceContainer, Normal, Normal, Allow),
    (ReopenInContainer, Normal, Normal, Allow),
    (DeleteContainer, Normal, Normal, Allow),
    (DeleteSpace, Normal, Normal, Allow),
    (MarkAgentSpace, Normal, Normal, Allow),
    (OpenInSpace, Normal, Normal, Allow),
    (ExtensionTab, Normal, Normal, Allow),
    (OpenLittle, Normal, Normal, Allow),
    (PromoteLittle, Normal, Normal, Allow),
];

/// What `reach` from `src` to `dst` does: the table's row, else refused.
pub fn crossing(reach: Reach, src: Side, dst: Side) -> Crossing {
    CROSSING
        .iter()
        .find(|(r, s, d, _)| (*r, *s, *d) == (reach, src, dst))
        .map_or(Crossing::Refuse, |&(.., c)| c)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [Reach; 13] = [
        MoveTab,
        MoveToWindow,
        SwitchSpace,
        NewTab,
        SetSpaceContainer,
        ReopenInContainer,
        DeleteContainer,
        DeleteSpace,
        MarkAgentSpace,
        OpenInSpace,
        ExtensionTab,
        OpenLittle,
        PromoteLittle,
    ];

    /// Every reach x side pair: only these two moves stay within the private side, and only a
    /// private window's Space switch goes to a normal window; nothing else touches it.
    #[test]
    fn the_private_line_is_crossed_by_nothing() {
        let within = [MoveTab, MoveToWindow, NewTab];
        for reach in ALL {
            for (src, dst) in [(Normal, Normal), (Normal, Private), (Private, Normal), (Private, Private)] {
                let want = match (src, dst) {
                    (Normal, Normal) => Allow,
                    (Private, Private) if within.contains(&reach) => Allow,
                    (Private, Normal) if reach == SwitchSpace => ToNormalWindow,
                    _ => Crossing::Refuse,
                };
                assert_eq!(crossing(reach, src, dst), want, "{reach:?} {src:?} -> {dst:?}");
            }
        }
    }
}
