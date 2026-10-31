//! The static menu tree of the Aligners module.
//!
//! One [`Node`] per menu entry is the single source of truth for the menu
//! ([`menu_tree`]), the command list ([`command_descs`]) and the inspector
//! (title and description by item id). The data is in `tree_data.rs`.
//!
//! Naming: a leaf is the command `aligners.<path>` (the path is the ids of
//! the group, item and optional sub-item joined by dots, e.g.
//! `aligners.preparation.segmentation.manual-brush`). Fluent ids cannot
//! contain dots, so the i18n keys use hyphens:
//!
//! | what        | id / key                                              |
//! |-------------|-------------------------------------------------------|
//! | command id  | `aligners.preparation.segmentation.manual-brush`      |
//! | label key   | `aligners-menu-preparation-segmentation-manual-brush` |
//! | description | `aligners-desc-preparation-segmentation-manual-brush` |
//!
//! Groups and sub-menus follow the same scheme (`aligners.preparation`,
//! `aligners-menu-preparation`) but are not commands.
//!
//! Status: a leaf is `Ready` if and only if a command with its id is
//! registered in [`crate::commands`]; otherwise it is a placeholder (`Stub`).

use dc_module_api::{CommandDesc, MenuItem, MenuStatus, MenuTree};

use crate::tree_data::GROUPS;

/// One entry of the menu: a group or sub-menu (with children) or a leaf
/// command.
#[derive(Debug, PartialEq, Eq)]
pub struct Node {
    /// Unique id; for a leaf it is also the command id.
    pub id: &'static str,
    /// i18n key of the label.
    pub label_key: &'static str,
    /// i18n key of the description shown in the inspector.
    pub desc_key: &'static str,
    /// Phosphor icon name (kebab case, e.g. `upload-simple`).
    pub icon: &'static str,
    /// Keyboard shortcut of a leaf, egui notation (`A`, `Ctrl+Shift+S`).
    pub shortcut: Option<&'static str>,
    /// Children; empty for a leaf.
    pub children: &'static [Node],
}

impl Node {
    /// A command; it is a placeholder until its id is registered in
    /// [`crate::commands`].
    pub(crate) const fn leaf(
        id: &'static str,
        label_key: &'static str,
        desc_key: &'static str,
        icon: &'static str,
    ) -> Self {
        Self {
            id,
            label_key,
            desc_key,
            icon,
            shortcut: None,
            children: &[],
        }
    }

    /// The same leaf with a keyboard shortcut.
    pub(crate) const fn key(mut self, shortcut: &'static str) -> Self {
        self.shortcut = Some(shortcut);
        self
    }

    /// A group or sub-menu.
    pub(crate) const fn group(
        id: &'static str,
        label_key: &'static str,
        desc_key: &'static str,
        icon: &'static str,
        children: &'static [Node],
    ) -> Self {
        Self {
            id,
            label_key,
            desc_key,
            icon,
            shortcut: None,
            children,
        }
    }

    /// `true` for a command (no children).
    pub fn is_leaf(&self) -> bool {
        self.children.is_empty()
    }

    /// A leaf is `Ready` when its command is registered, `Stub` otherwise;
    /// a group is `Ready` as soon as one of its descendants is, `Stub` while
    /// all of them are placeholders.
    pub fn status(&self) -> MenuStatus {
        if self.is_leaf() {
            if crate::commands::is_registered(self.id) {
                MenuStatus::Ready
            } else {
                MenuStatus::Stub
            }
        } else if self
            .children
            .iter()
            .any(|c| c.status() == MenuStatus::Ready)
        {
            MenuStatus::Ready
        } else {
            MenuStatus::Stub
        }
    }

    fn to_menu_item(&self) -> MenuItem {
        let item = if self.is_leaf() {
            MenuItem::command(self.id, self.label_key, self.id)
        } else {
            MenuItem::submenu(
                self.id,
                self.label_key,
                self.children.iter().map(Node::to_menu_item).collect(),
            )
        };
        let item = item.with_icon(self.icon);
        if self.status() == MenuStatus::Stub {
            item.stub()
        } else {
            item
        }
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a Node>) {
        out.push(self);
        for c in self.children {
            c.collect(out);
        }
    }
}

/// The top-level groups.
pub fn groups() -> &'static [Node] {
    GROUPS
}

/// Every node, depth first, parents before children.
pub fn all_nodes() -> Vec<&'static Node> {
    let mut out = Vec::new();
    for g in GROUPS {
        g.collect(&mut out);
    }
    out
}

/// Every leaf (command), in menu order.
pub fn leaves() -> Vec<&'static Node> {
    all_nodes().into_iter().filter(|n| n.is_leaf()).collect()
}

/// Node by id, at any depth.
pub fn find(id: &str) -> Option<&'static Node> {
    all_nodes().into_iter().find(|n| n.id == id)
}

/// The menu shown in the workspace's left panel.
pub fn menu_tree() -> MenuTree {
    MenuTree::new(GROUPS.iter().map(Node::to_menu_item).collect())
}

/// The commands of the module: one per leaf, with the description as
/// tooltip.
pub fn command_descs() -> Vec<CommandDesc> {
    leaves()
        .into_iter()
        .map(|n| {
            let mut c = CommandDesc::new(n.id, n.label_key);
            c.tooltip_key = Some(n.desc_key);
            c.shortcut = n.shortcut;
            c.status = n.status();
            c
        })
        .collect()
}
