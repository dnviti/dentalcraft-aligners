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
    for leaf in tree::leaves() {
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
