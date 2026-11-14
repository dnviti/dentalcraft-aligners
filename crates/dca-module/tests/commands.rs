//! Invariants of the command registry (`dca_module::commands`): the menu
//! status follows it, and every registered command is documented in both
//! languages and has a parameter schema.

use std::collections::BTreeSet;
use std::path::PathBuf;

use dc_i18n::{Lang, parity};
use dc_module_api::{MenuStatus, Module};
use dca_module::{AlignersModule, commands, tree};

fn ftl_ids(lang: Lang) -> BTreeSet<String> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("i18n")
        .join(lang.code())
        .join("dca-module.ftl");
    parity::ids(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn a_leaf_is_ready_if_and_only_if_its_command_is_registered() {
    let module = AlignersModule::new();
    for leaf in tree::leaves().into_iter().chain(tree::tools()) {
        let expected = if commands::is_registered(leaf.id) {
            MenuStatus::Ready
        } else {
            MenuStatus::Stub
        };
        assert_eq!(leaf.status(), expected, "leaf {}", leaf.id);
    }
    for c in module.commands() {
        assert_eq!(
            c.status == MenuStatus::Ready,
            commands::is_registered(c.id),
            "command {}",
            c.id
        );
    }
    let menu = module.menu();
    for (_, item) in menu.iter() {
        match item.command_id() {
            Some(id) => assert_eq!(
                item.status == MenuStatus::Ready,
                commands::is_registered(id),
                "menu item {}",
                item.id
            ),
            // a group is Ready as soon as one of its commands is
            None => {
                let node = tree::find(item.id).unwrap();
                assert_eq!(item.status, node.status(), "group {}", item.id);
                let any_ready = descendants(node).any(|n| commands::is_registered(n.id));
                assert_eq!(item.status == MenuStatus::Ready, any_ready, "{}", item.id);
            }
        }
    }
}

fn descendants(node: &'static tree::Node) -> impl Iterator<Item = &'static tree::Node> {
    let mut stack: Vec<&'static tree::Node> = node.children.iter().collect();
    std::iter::from_fn(move || {
        let n = stack.pop()?;
        stack.extend(n.children.iter());
        Some(n)
    })
}

/// The commands that left the menu: CLI and MCP only, ids unchanged (the
/// manual tools, [`tree::tools`], have a node of their own).
const OUT_OF_THE_MENU: &[&str] = &[
    "aligners.frame",
    "aligners.demo",
    "aligners.case.detect-arch",
    // the scan corrections of the Import scans inspector
    "aligners.case.set-arch",
    "aligners.case.make-base",
    "aligners.preparation.orientation.automatic",
    "aligners.preparation.numbering.automatic",
    "aligners.teeth.frames",
    "aligners.preparation.reconstruction.automatic",
    "aligners.preparation.reconstruction.rebuild-tooth",
    "aligners.manual.rebuild-tooth",
    "aligners.manual.tooth-region",
    "aligners.preparation.numbering.sequence",
    "aligners.preparation.numbering.clear-seeds",
    "aligners.preparation.gingiva",
    "aligners.preparation.segmentation.check",
    "aligners.diagnosis.little-index",
    "aligners.diagnosis.bolton",
    "aligners.diagnosis.complexity",
    // the manual corrections of the Case analysis inspector
    "aligners.diagnosis.edit-values",
    "aligners.planning.staging.automatic",
    "aligners.planning.schedule",
    "aligners.occlusion.virtual-bite",
    "aligners.occlusion.bite-registration.icp",
    // the correction button of the Bite inspector (Three points is a tool)
    "aligners.occlusion.bite-correct",
    "aligners.occlusion.contact-map",
    "aligners.occlusion.bite-with-aligners",
    "aligners.occlusion.bite-ramps",
    "aligners.production.base-labels",
    "aligners.production.shell.automatic",
    "aligners.production.blockout",
    "aligners.production.trim-line",
    "aligners.production.stamp",
    "aligners.output.export-stl",
    "aligners.output.print-check",
    "aligners.output.manifest",
    "aligners.output.report",
    "aligners.settings.study-models",
    "aligners.settings.find-teeth",
    "aligners.settings.segmentation",
    // the gizmo of the viewport on a selected 3D component
    "aligners.components.transform",
];

#[test]
fn what_left_the_menu_stays_registered_with_its_own_label() {
    let en = ftl_ids(Lang::En);
    for id in OUT_OF_THE_MENU {
        let spec = commands::find(id).unwrap_or_else(|| panic!("{id} is not registered"));
        assert!(tree::find(id).is_none(), "{id} is in the menu");
        assert!(
            spec.label_key.starts_with("aligners-cmd-"),
            "{id}: {}",
            spec.label_key
        );
        assert!(en.contains(spec.label_key), "{id}");
    }
    // every command is a leaf of the menu, a tool or one of these: nothing is lost
    let leaves = tree::leaves();
    assert_eq!(
        commands::all().len(),
        leaves.len() + tree::tools().len() + OUT_OF_THE_MENU.len(),
        "a command is neither in the menu, nor a tool, nor in OUT_OF_THE_MENU"
    );
}

#[test]
fn a_leaf_or_a_tool_and_its_command_have_the_same_label() {
    for leaf in tree::leaves().into_iter().chain(tree::tools()) {
        let spec = commands::find(leaf.id).unwrap_or_else(|| panic!("{} has no command", leaf.id));
        assert_eq!(spec.label_key, leaf.label_key, "{}", leaf.id);
    }
}

#[test]
fn registered_commands_are_unique_documented_and_have_a_schema() {
    let en = ftl_ids(Lang::En);
    let it = ftl_ids(Lang::It);
    let specs = commands::all();
    let ids: BTreeSet<_> = specs.iter().map(|c| c.id).collect();
    assert_eq!(ids.len(), specs.len(), "duplicate command ids");
    for c in &specs {
        assert!(c.id.starts_with("aligners."), "{}", c.id);
        for key in [c.label_key, c.doc_key] {
            assert!(en.contains(key), "{}: {key} missing in en", c.id);
            assert!(it.contains(key), "{}: {key} missing in it", c.id);
        }
        let schema = c.schema();
        assert!(
            schema.is_object() && schema["type"] == "object",
            "{}: parameters schema is not an object schema: {schema}",
            c.id
        );
    }
}

#[test]
fn the_module_exposes_the_registry() {
    let from_module: Vec<_> = AlignersModule::new()
        .command_specs()
        .iter()
        .map(|c| c.id)
        .collect();
    let from_registry: Vec<_> = commands::all().iter().map(|c| c.id).collect();
    assert_eq!(from_module, from_registry);
}
