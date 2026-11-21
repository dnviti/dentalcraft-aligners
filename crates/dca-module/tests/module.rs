//! Tests of the Aligners module: menu tree, translations, registration and
//! inspector.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use dc_case::{CaseState, ScanRole};
use dc_i18n::{Lang, parity};
use dc_jobs::JobSystem;
use dc_module_api::{
    LeafInput, MenuStatus, Module, ModuleCtx, ModuleError, ModuleRegistry, NOT_AVAILABLE_KEY,
};
use dca_module::{AlignersModule, MODULE_ID, NAME_KEY, loader, tree};

fn i18n_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("i18n")
}

fn ftl_ids(lang: Lang) -> BTreeSet<String> {
    let path = i18n_dir().join(lang.code()).join("dca-module.ftl");
    parity::ids(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn ids_are_unique_and_follow_the_naming_scheme() {
    let nodes = tree::all_nodes();
    let ids: BTreeSet<_> = nodes.iter().map(|n| n.id).collect();
    assert_eq!(ids.len(), nodes.len(), "duplicate node ids");

    let menu = AlignersModule::new().menu();
    assert!(menu.duplicate_ids().is_empty());

    for n in &nodes {
        let path =
            n.id.strip_prefix("aligners.")
                .expect("id starts with aligners.");
        assert!(!path.is_empty());
        // Fluent ids have no dots: label and description keys use hyphens
        let k = path.replace('.', "-");
        assert_eq!(n.label_key, format!("aligners-menu-{k}"), "{}", n.id);
        assert_eq!(n.desc_key, format!("aligners-desc-{k}"), "{}", n.id);
        assert!(
            !n.icon.is_empty()
                && n.icon
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "icon of {} is not a Phosphor name: {:?}",
            n.id,
            n.icon
        );
    }
}

// The Ready / Stub status of leaves and groups follows the command registry:
// see tests/commands.rs.
#[test]
fn every_leaf_and_every_tool_is_a_unique_command() {
    let module = AlignersModule::new();
    let leaves = tree::leaves();
    assert_eq!(leaves.len(), TREE.len(), "the agreed tree");

    let command_ids: Vec<_> = module.commands().iter().map(|c| c.id).collect();
    let unique: BTreeSet<_> = command_ids.iter().collect();
    assert_eq!(unique.len(), command_ids.len(), "duplicate command ids");
    // the leaves in menu order, then the tools of the inspectors
    let (menu, tools) = command_ids.split_at(leaves.len());
    assert_eq!(menu, module.menu().command_ids());
    assert_eq!(menu, leaves.iter().map(|n| n.id).collect::<Vec<_>>());
    assert_eq!(
        tools,
        tree::tools().iter().map(|n| n.id).collect::<Vec<_>>()
    );
    for c in module.commands() {
        assert!(c.id.starts_with("aligners."));
        assert!(c.tooltip_key.is_some());
    }
    for (_, item) in module
        .menu()
        .iter()
        .filter(|(_, i)| i.command_id().is_some())
    {
        assert_eq!(
            item.command_id(),
            Some(item.id),
            "menu id is the command id"
        );
    }
}

#[test]
fn no_empty_groups_and_the_spec_groups_exist() {
    let groups = tree::groups();
    let ids: Vec<_> = groups.iter().map(|g| g.id).collect();
    assert_eq!(
        ids,
        [
            "aligners.case",
            "aligners.teeth",
            "aligners.diagnosis",
            "aligners.planning",
            "aligners.production",
            "aligners.followup",
        ]
    );
    for n in tree::all_nodes() {
        if !n.is_leaf() {
            assert!(!n.children.is_empty(), "empty group {}", n.id);
        }
    }
    for g in groups {
        assert!(!g.children.is_empty(), "empty group {}", g.id);
    }
    // two levels: the groups and their functions
    assert_eq!(module_depth(), 1);
}

fn module_depth() -> usize {
    AlignersModule::new()
        .menu()
        .iter()
        .map(|(d, _)| d)
        .max()
        .unwrap()
}

/// The menu, in order: group, leaf, English and Italian label.
const TREE: &[(&str, &str, &str, &str)] = &[
    (
        "aligners.case",
        "aligners.case.import-scans",
        "Import scans",
        "Importa scansioni",
    ),
    (
        "aligners.case",
        "aligners.auto",
        "Run everything automatically",
        "Esegui tutto in automatico",
    ),
    (
        "aligners.teeth",
        "aligners.teeth.find",
        "Find the teeth",
        "Trova i denti",
    ),
    (
        "aligners.teeth",
        "aligners.preparation.segmentation.automatic",
        "Separate teeth and gum",
        "Separa denti e gengiva",
    ),
    (
        "aligners.teeth",
        "aligners.teeth.build",
        "3D teeth",
        "Denti 3D",
    ),
    (
        "aligners.diagnosis",
        "aligners.diagnosis.space-analysis",
        "Case analysis",
        "Analisi del caso",
    ),
    (
        "aligners.planning",
        "aligners.occlusion.align",
        "Bite",
        "Morso",
    ),
    (
        "aligners.planning",
        "aligners.planning.setup.automatic",
        "Final position",
        "Posizione finale",
    ),
    (
        "aligners.planning",
        "aligners.planning.aligners",
        "Aligners",
        "Allineatori",
    ),
    (
        "aligners.planning",
        "aligners.review.classify",
        "Clinical review",
        "Revisione clinica",
    ),
    (
        "aligners.production",
        "aligners.output.export-all",
        "Export for printing",
        "Esporta per la stampa",
    ),
    (
        "aligners.production",
        "aligners.output.study-models",
        "Study models",
        "Modelli da studio",
    ),
    (
        "aligners.followup",
        "aligners.followup.visits",
        "Control visit",
        "Visita di controllo",
    ),
    (
        "aligners.followup",
        "aligners.followup.tracking",
        "Re-plan",
        "Ripianifica",
    ),
];

#[test]
fn the_menu_is_the_agreed_tree() {
    let leaves: Vec<_> = tree::leaves().iter().map(|n| n.id).collect();
    let agreed: Vec<_> = TREE.iter().map(|(_, id, ..)| *id).collect();
    assert_eq!(leaves, agreed);
    let menu = AlignersModule::new().menu();
    for (group, id, ..) in TREE {
        let path = menu.path_to(id).unwrap();
        assert_eq!(path[path.len() - 2], *group, "{id}");
    }
    // no sub-group: the corrections are tools of the inspectors, not entries
    assert_eq!(module_depth(), 1);
    for tool in tree::tools() {
        assert!(menu.find(tool.id).is_none(), "{} is in the menu", tool.id);
    }
}

#[test]
fn the_manual_tools_are_registered_commands_out_of_the_menu() {
    let tools: Vec<_> = tree::tools().iter().map(|n| n.id).collect();
    assert_eq!(
        tools,
        [
            "aligners.preparation.numbering.place-seeds",
            "aligners.preparation.numbering.delete-seed",
            "aligners.preparation.numbering.renumber",
            "aligners.preparation.numbering.mark-missing",
            "aligners.preparation.segmentation.manual-brush",
            "aligners.preparation.segmentation.clean-borders",
            "aligners.planning.movements",
            "aligners.planning.setup.line",
            "aligners.occlusion.bite-registration.three-points",
        ]
    );
    for tool in tree::tools() {
        assert!(tool.is_leaf(), "{}", tool.id);
        assert_eq!(tool.status(), MenuStatus::Ready, "{}", tool.id);
        assert!(tree::find(tool.id).is_none(), "{}", tool.id);
        // the CLI and the MCP server list and run it with its parameters
        let spec = dca_module::commands::find(tool.id).unwrap();
        assert_eq!(spec.label_key, tool.label_key, "{}", tool.id);
        assert_eq!(spec.schema()["type"], "object", "{}", tool.id);
    }
    // a click never runs a tool: invoking it is refused, nothing is queued
    let jobs = JobSystem::new(1).unwrap();
    let state = Arc::new(CaseState::default());
    let mut ctx = ModuleCtx::new(&state, &jobs);
    let mut m = AlignersModule::new();
    for id in tools {
        assert!(m.invoke(id, &mut ctx).is_err(), "{id}");
    }
    let (cmds, toasts) = ctx.finish();
    assert!(cmds.is_empty() && toasts.is_empty());
}

#[test]
fn the_shell_is_told_what_to_ask_before_a_leaf_runs() {
    let menu = AlignersModule::new().menu();
    let asking: Vec<_> = menu
        .iter()
        .filter(|(_, i)| i.input.is_some())
        .map(|(_, i)| (i.id, i.input.unwrap()))
        .collect();
    assert_eq!(
        asking,
        [
            (
                "aligners.output.export-all",
                LeafInput::Folder { param: "dir" }
            ),
            (
                "aligners.followup.visits",
                LeafInput::ScanFile { param: "scan" }
            ),
        ]
    );
    for (id, input) in asking {
        let (LeafInput::Folder { param } | LeafInput::ScanFile { param }) = input;
        let schema = dca_module::commands::find(id).unwrap().schema();
        assert!(schema["properties"][param].is_object(), "{id}: {schema}");
    }
}

#[test]
fn only_import_scans_offers_the_scanner_with_every_role() {
    let menu = AlignersModule::new().menu();
    let scanning: Vec<_> = menu
        .iter()
        .filter_map(|(_, i)| i.scanner.map(|s| (i.id, s.roles)))
        .collect();
    assert_eq!(
        scanning,
        [(
            "aligners.case.import-scans",
            &[ScanRole::Upper, ScanRole::Lower, ScanRole::Bite][..]
        )]
    );
    assert!(dc_module_api::is_import_scans_command(scanning[0].0));
}

/// What the menu shows as the progress of the case: the nine steps of the
/// workflow, in menu order. Everything else is a shortcut, an export or a
/// follow-up (the corrections are tools of the inspectors, not entries).
const STEPS: [&str; 9] = [
    "aligners.case.import-scans",
    "aligners.teeth.find",
    "aligners.preparation.segmentation.automatic",
    "aligners.teeth.build",
    "aligners.diagnosis.space-analysis",
    "aligners.occlusion.align",
    "aligners.planning.setup.automatic",
    "aligners.planning.aligners",
    "aligners.review.classify",
];

#[test]
fn nine_steps_count_and_nothing_else() {
    let menu = AlignersModule::new().menu();
    assert_eq!(menu.step_ids(), STEPS);
    for not_a_step in [
        "aligners.auto",
        "aligners.output.export-all",
        "aligners.output.study-models",
        "aligners.followup.tracking",
        "aligners.followup.visits",
    ] {
        assert!(!menu.step_ids().contains(&not_a_step), "{not_a_step}");
    }
}

#[test]
fn the_module_reads_the_steps_from_the_case() {
    let m = AlignersModule::new();
    let empty = CaseState::default();
    assert!(!m.leaf_done(&empty, "aligners.case.import-scans"));
    let demo = dca_module::commands::find("aligners.demo").unwrap();
    let (state, _) = demo
        .execute(
            &empty,
            &serde_json::json!({ "arch": "lower" }),
            &dc_command::JobCtx::detached(),
        )
        .unwrap();
    assert!(m.leaf_done(&state, "aligners.case.import-scans"));
    assert!(m.leaf_done(&state, "aligners.teeth.find"));
    assert!(!m.leaf_done(&state, "aligners.planning.aligners"));
    assert!(!m.leaf_done(&state, "aligners.auto"));
}

#[test]
fn translations_cover_every_key_in_both_languages() {
    // same files, same keys in en and it
    parity::check_dir(&i18n_dir()).unwrap();

    let en = ftl_ids(Lang::En);
    let it = ftl_ids(Lang::It);
    assert_eq!(en, it);

    let mut used: BTreeSet<String> = BTreeSet::new();
    for n in tree::all_nodes().into_iter().chain(tree::tools()) {
        for key in [n.label_key, n.desc_key] {
            assert!(en.contains(key), "{key} missing in en");
            assert!(it.contains(key), "{key} missing in it");
            used.insert(key.to_owned());
        }
    }
    // no orphan menu keys left behind
    for key in en
        .iter()
        .filter(|k| k.starts_with("aligners-menu-") || k.starts_with("aligners-desc-"))
    {
        assert!(
            used.contains(key),
            "{key} is used by no menu entry and no tool"
        );
    }
    assert!(en.contains(NAME_KEY) && en.contains("aligners-status-stub"));
}

// One test: the language is process-wide state.
#[test]
fn labels_resolve_in_english_and_italian() {
    loader();
    dc_i18n::set_language(Lang::En).unwrap();
    assert_eq!(dc_i18n::tr(NAME_KEY), "Aligners");
    assert_eq!(dc_i18n::tr("aligners-menu-case"), "Case");
    for (key, en, _) in GROUP_LABELS {
        assert_eq!(dc_i18n::tr(key), *en);
    }
    assert_eq!(dc_i18n::tr(NOT_AVAILABLE_KEY), "Not available yet");
    for n in tree::all_nodes().into_iter().chain(tree::tools()) {
        for key in [n.label_key, n.desc_key] {
            let text = dc_i18n::tr(key);
            assert!(!text.trim().is_empty() && text != key, "en {key}");
        }
    }
    for (_, id, en, _) in TREE {
        assert_eq!(dc_i18n::tr(tree::find(id).unwrap().label_key), *en, "{id}");
    }
    // an error of the automatic seeds is a sentence of the app, not the English of the algorithm
    let refused_ends = || -> String {
        let spec = dca_module::commands::find("aligners.preparation.numbering.automatic").unwrap();
        spec.execute(
            &CaseState::default(),
            &serde_json::json!({ "ends": [4, 7] }),
            &dc_command::JobCtx::detached(),
        )
        .unwrap_err()
        .to_string()
    };
    let spec_labels = || -> Vec<(String, &str)> {
        dca_module::commands::all()
            .iter()
            .map(|c| (dc_i18n::tr(c.label_key), c.id))
            .collect()
    };
    assert_unique(spec_labels());
    assert!(refused_ends().contains("5, 6, 7 or 8"));

    dc_i18n::set_language(Lang::It).unwrap();
    assert_eq!(dc_i18n::tr(NAME_KEY), "Allineatori");
    for (key, _, it) in GROUP_LABELS {
        assert_eq!(dc_i18n::tr(key), *it);
    }
    for n in tree::all_nodes().into_iter().chain(tree::tools()) {
        for key in [n.label_key, n.desc_key] {
            let text = dc_i18n::tr(key);
            assert!(!text.trim().is_empty() && text != key, "it {key}");
        }
    }
    for (_, id, _, it) in TREE {
        assert_eq!(dc_i18n::tr(tree::find(id).unwrap().label_key), *it, "{id}");
    }
    assert_unique(spec_labels());
    assert!(refused_ends().contains("5, 6, 7 o 8"));

    // the inspector of a group shows its translated title and description
    // (the shell draws the ones of a leaf)
    let group = tree::find("aligners.followup").unwrap();
    let texts = render_inspector_texts(Some(group.id));
    for key in [group.label_key, group.desc_key] {
        assert!(texts.contains(&dc_i18n::tr(key)), "{texts:?}");
    }

    dc_i18n::set_language(Lang::En).unwrap();
}

/// No two commands share a label: the CLI and the MCP server list them by it.
fn assert_unique(labels: Vec<(String, &str)>) {
    let mut seen = std::collections::BTreeMap::new();
    for (label, id) in labels {
        if let Some(other) = seen.insert(label.clone(), id) {
            panic!("{id} and {other} are both labelled {label:?}");
        }
    }
}

const GROUP_LABELS: &[(&str, &str, &str)] = &[
    ("aligners-menu-case", "Case", "Caso"),
    ("aligners-menu-teeth", "Teeth", "Denti"),
    ("aligners-menu-diagnosis", "Diagnosis", "Diagnosi"),
    ("aligners-menu-planning", "Plan", "Piano"),
    ("aligners-menu-production", "Production", "Produzione"),
    ("aligners-menu-followup", "Follow-up", "Visite"),
];

#[test]
fn registers_in_a_module_registry() {
    let mut reg = ModuleRegistry::new();
    dca_module::register(&mut reg).unwrap();
    assert!(matches!(
        dca_module::register(&mut reg),
        Err(ModuleError::Duplicate(_))
    ));
    assert_eq!(reg.ids(), vec![MODULE_ID]);
    assert_eq!(MODULE_ID, "aligners");
    let m = reg.get("aligners").unwrap();
    assert_eq!(m.name_key(), NAME_KEY);
    assert_eq!(m.version(), env!("CARGO_PKG_VERSION"));
    assert!(reg.is_enabled("aligners"));
    assert_eq!(m.menu().roots.len(), 6);
}

#[test]
fn every_leaf_works_and_only_leaves_can_be_invoked() {
    let jobs = JobSystem::new(1).unwrap();
    let state = Arc::new(CaseState::default());
    let mut ctx = ModuleCtx::new(&state, &jobs);
    let mut m = AlignersModule::new();
    assert_eq!(m.selected(), None);
    for n in tree::leaves() {
        assert_eq!(n.status(), MenuStatus::Ready, "{}", n.id);
    }
    // groups are not commands; unknown ids are errors
    assert!(m.invoke("aligners.planning", &mut ctx).is_err());
    assert!(m.invoke("aligners.nope", &mut ctx).is_err());
    // a command out of the menu runs from the CLI, not from a click
    assert!(m.invoke("aligners.planning.schedule", &mut ctx).is_err());
    assert!(!m.select_entry("aligners.nope"));
    assert!(m.select_entry("aligners.planning"));
    let (cmds, toasts) = ctx.finish();
    assert!(cmds.is_empty());
    assert!(toasts.is_empty());
}

#[test]
fn a_leaf_with_an_input_waits_for_the_shell_and_the_others_run_with_their_defaults() {
    let jobs = JobSystem::new(1).unwrap();
    let state = Arc::new(CaseState::default());
    let mut m = AlignersModule::new();
    for id in ["aligners.output.export-all", "aligners.followup.visits"] {
        let mut ctx = ModuleCtx::new(&state, &jobs);
        m.invoke(id, &mut ctx).unwrap();
        assert_eq!(m.selected(), Some(id));
        let (cmds, toasts) = ctx.finish();
        assert!(cmds.is_empty(), "{id}");
        assert!(toasts.is_empty(), "{id}");
    }
    for id in ["aligners.auto", "aligners.followup.tracking"] {
        let mut ctx = ModuleCtx::new(&state, &jobs);
        m.invoke(id, &mut ctx).unwrap();
        let (cmds, _) = ctx.finish();
        assert_eq!(cmds.len(), 1, "{id}");
        assert_eq!(cmds[0].id(), id);
    }
}

#[test]
fn a_click_on_the_bite_queues_the_command_and_says_nothing_before_it_runs() {
    // what the bite rests on is told by its output, after it ran
    let jobs = JobSystem::new(1).unwrap();
    let state = Arc::new(CaseState::default());
    let mut m = AlignersModule::new();
    let mut ctx = ModuleCtx::new(&state, &jobs);
    m.invoke("aligners.occlusion.align", &mut ctx).unwrap();
    let (cmds, toasts) = ctx.finish();
    assert_eq!(cmds.len(), 1);
    assert!(toasts.is_empty());
}

#[test]
fn invoking_the_import_leaf_runs_nothing_the_shell_asks_for_the_files() {
    let jobs = JobSystem::new(1).unwrap();
    let state = Arc::new(CaseState::default());
    let mut ctx = ModuleCtx::new(&state, &jobs);
    let mut m = AlignersModule::new();
    let id = "aligners.case.import-scans";
    assert_eq!(tree::find(id).unwrap().status(), MenuStatus::Ready);
    m.invoke(id, &mut ctx).unwrap();
    assert_eq!(m.selected(), Some(id));
    let (cmds, toasts) = ctx.finish();
    assert!(cmds.is_empty());
    assert!(toasts.is_empty());
}

#[test]
fn codec_round_trips_the_state() {
    let m = AlignersModule::new();
    let codec = m.data_codec();
    assert_eq!(codec.current_version(), dca_model::SCHEMA_VERSION);
    let state = AlignersModule::initial_state();
    let json = state.to_json().unwrap();
    let back = codec.load(json, dca_model::SCHEMA_VERSION).unwrap();
    assert_eq!(
        back.as_any().downcast_ref::<dca_model::AlignersState>(),
        Some(&dca_model::AlignersState::default())
    );
    // newer than the module understands, or no migration known
    assert!(
        codec
            .load(serde_json::json!({}), dca_model::SCHEMA_VERSION + 1)
            .is_err()
    );
    assert!(codec.load(serde_json::json!({}), 0).is_err());
}

#[test]
fn shell_selection_reaches_the_module() {
    let mut m = AlignersModule::new();
    Module::select(&mut m, "aligners.planning");
    assert_eq!(m.selected(), Some("aligners.planning"));
    Module::select(&mut m, "aligners.nope");
    assert_eq!(m.selected(), Some("aligners.planning"));
}

#[test]
fn design_shortcuts_are_declared() {
    let m = AlignersModule::new();
    let cmds = m.commands();
    let key = |id: &str| cmds.iter().find(|c| c.id == id).unwrap().shortcut;
    assert_eq!(
        key("aligners.preparation.segmentation.automatic"),
        Some("A")
    );
    assert_eq!(
        key("aligners.preparation.segmentation.manual-brush"),
        Some("B")
    );
    assert_eq!(key("aligners.planning.aligners"), Some("S"));
    assert_eq!(cmds.iter().filter(|c| c.shortcut.is_some()).count(), 3);
}

#[test]
fn scene_is_empty_for_now() {
    assert!(
        AlignersModule::new()
            .scene(&CaseState::default())
            .is_empty()
    );
}

fn run_inspector(selected: Option<&str>) -> egui::FullOutput {
    let jobs = JobSystem::new(1).unwrap();
    let state = Arc::new(CaseState::default());
    let mut module = AlignersModule::new();
    if let Some(id) = selected {
        assert!(module.select_entry(id), "unknown item {id}");
    }
    let egui_ctx = egui::Context::default();
    // everything the module can draw: the advanced options on
    dc_module_api::set_advanced_ui(&egui_ctx, true);
    let mut out = egui_ctx.run_ui(egui::RawInput::default(), |ui| {
        let mut ctx = ModuleCtx::new(&state, &jobs);
        module.inspector(ui, &mut ctx);
    });
    // egui asserts that texture updates are consumed (there is no renderer here)
    out.textures_delta.clear();
    out
}

fn render_inspector_texts(selected: Option<&str>) -> Vec<String> {
    run_inspector(selected)
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

/// The leaves whose parameters the module draws in the inspector.
const WITH_PARAMETERS: [&str; 2] = ["aligners.output.study-models", "aligners.teeth.find"];

#[test]
fn inspector_renders_every_item_without_panicking() {
    let out = run_inspector(None);
    assert!(!out.shapes.is_empty());
    for n in tree::all_nodes() {
        let texts = render_inspector_texts(Some(n.id));
        // a group shows its title and description; a leaf only its
        // parameters, the shell draws the rest of its inspector
        if n.is_leaf() && !WITH_PARAMETERS.contains(&n.id) {
            assert!(texts.is_empty(), "{}: {texts:?}", n.id);
        } else {
            assert!(!texts.is_empty(), "nothing drawn for {}", n.id);
        }
    }
}

#[test]
fn no_page_shows_controls_that_do_nothing() {
    // the staging placeholder was a page of disabled sliders: only the study
    // models and Find the teeth have settings
    for n in tree::all_nodes() {
        let out = run_inspector(Some(n.id));
        let inputs = out
            .shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Rect(_)))
            .count();
        let texts = render_inspector_texts(Some(n.id)).join("\n");
        assert!(!texts.contains("0.33"), "{}: {texts}", n.id);
        assert!(!texts.contains("0.25"), "{}: {texts}", n.id);
        if !WITH_PARAMETERS.contains(&n.id) {
            assert_eq!(inputs, 0, "{}", n.id);
        }
    }
}
