//! Tests of the Aligners module: menu tree, translations, registration and
//! inspector.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use dc_case::CaseState;
use dc_i18n::{Lang, parity};
use dc_jobs::JobSystem;
use dc_module_api::{
    MenuStatus, Module, ModuleCtx, ModuleError, ModuleRegistry, NOT_AVAILABLE_KEY, Toast,
    ToastLevel,
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

#[test]
fn every_leaf_is_a_unique_stub_command() {
    let module = AlignersModule::new();
    let leaves = tree::leaves();
    assert!(leaves.len() >= 40, "only {} leaves", leaves.len());

    let command_ids: Vec<_> = module.commands().iter().map(|c| c.id).collect();
    let unique: BTreeSet<_> = command_ids.iter().collect();
    assert_eq!(unique.len(), command_ids.len(), "duplicate command ids");
    assert_eq!(command_ids, module.menu().command_ids());
    assert_eq!(command_ids, leaves.iter().map(|n| n.id).collect::<Vec<_>>());
    for c in module.commands() {
        assert!(c.id.starts_with("aligners."));
        assert_eq!(c.status, MenuStatus::Stub, "{}", c.id);
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
        assert_eq!(item.status, MenuStatus::Stub);
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
            "aligners.preparation",
            "aligners.diagnosis",
            "aligners.planning",
            "aligners.occlusion",
            "aligners.production",
            "aligners.followup",
            "aligners.output",
            "aligners.tools",
        ]
    );
    for n in tree::all_nodes() {
        if !n.is_leaf() {
            assert!(!n.children.is_empty(), "empty group {}", n.id);
        }
    }
    for g in groups {
        assert!(!g.children.is_empty(), "empty group {}", g.id);
        assert!(g.status() == MenuStatus::Stub);
    }
    // the menu is multi-level
    assert!(module_depth() >= 2);
}

fn module_depth() -> usize {
    AlignersModule::new()
        .menu()
        .iter()
        .map(|(d, _)| d)
        .max()
        .unwrap()
}

#[test]
fn spec_items_are_present() {
    for id in [
        "aligners.case.import-scans",
        "aligners.preparation.segmentation.automatic",
        "aligners.preparation.segmentation.manual-brush",
        "aligners.diagnosis.little-index",
        "aligners.diagnosis.bolton",
        "aligners.planning.staging",
        "aligners.planning.ipr",
        "aligners.planning.attachments",
        "aligners.occlusion.bite-registration.icp",
        "aligners.occlusion.contact-map",
        "aligners.production.trim-line",
        "aligners.production.stamp",
        "aligners.followup.visits",
        "aligners.output.export-stl",
        "aligners.tools.manual-tools",
        "aligners.tools.history",
    ] {
        assert!(tree::find(id).is_some(), "missing {id}");
    }
}

#[test]
fn translations_cover_every_key_in_both_languages() {
    // same files, same keys in en and it
    parity::check_dir(&i18n_dir()).unwrap();

    let en = ftl_ids(Lang::En);
    let it = ftl_ids(Lang::It);
    assert_eq!(en, it);

    let mut used: BTreeSet<String> = BTreeSet::new();
    for n in tree::all_nodes() {
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
        assert!(used.contains(key), "{key} is not used by the menu tree");
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
    assert_eq!(dc_i18n::tr(NOT_AVAILABLE_KEY), "Not available yet");
    for n in tree::all_nodes() {
        for key in [n.label_key, n.desc_key] {
            let text = dc_i18n::tr(key);
            assert!(!text.trim().is_empty() && text != key, "en {key}");
        }
    }

    dc_i18n::set_language(Lang::It).unwrap();
    assert_eq!(dc_i18n::tr(NAME_KEY), "Allineatori");
    assert_eq!(
        dc_i18n::tr("aligners-menu-diagnosis-little-index"),
        "Indice di irregolarità di Little"
    );
    assert_eq!(dc_i18n::tr("aligners-menu-planning-ipr"), "Stripping (IPR)");
    assert_eq!(
        dc_i18n::tr("aligners-menu-production-trim-line"),
        "Linea di rifilatura"
    );
    for n in tree::all_nodes() {
        for key in [n.label_key, n.desc_key] {
            let text = dc_i18n::tr(key);
            assert!(!text.trim().is_empty() && text != key, "it {key}");
        }
    }

    // the inspector shows the translated "not available" notice
    let texts = render_inspector_texts(Some("aligners.planning.ipr"));
    assert!(
        texts.iter().any(|t| t == "Non ancora disponibile"),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t == "Stripping (IPR)"), "{texts:?}");

    dc_i18n::set_language(Lang::En).unwrap();
    let texts = render_inspector_texts(Some("aligners.planning.ipr"));
    assert!(texts.iter().any(|t| t == "Not yet available"), "{texts:?}");
}

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
    assert_eq!(m.menu().roots.len(), 9);
}

#[test]
fn invoking_a_stub_selects_it_and_shows_the_toast() {
    let jobs = JobSystem::new(1).unwrap();
    let state = Arc::new(CaseState::default());
    let mut ctx = ModuleCtx::new(&state, &jobs);
    let mut m = AlignersModule::new();
    assert_eq!(m.selected(), None);
    m.invoke("aligners.planning.staging.automatic", &mut ctx)
        .unwrap();
    assert_eq!(m.selected(), Some("aligners.planning.staging.automatic"));
    // groups are not commands; unknown ids are errors
    assert!(m.invoke("aligners.planning", &mut ctx).is_err());
    assert!(m.invoke("aligners.nope", &mut ctx).is_err());
    assert!(!m.select_entry("aligners.nope"));
    assert!(m.select_entry("aligners.planning"));
    let (cmds, toasts) = ctx.finish();
    assert!(cmds.is_empty());
    assert_eq!(
        toasts,
        vec![Toast::new(ToastLevel::Info, NOT_AVAILABLE_KEY)]
    );
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
    assert_eq!(key("aligners.planning.staging.automatic"), Some("S"));
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

#[test]
fn inspector_renders_every_item_without_panicking() {
    let out = run_inspector(None);
    assert!(!out.shapes.is_empty());
    for n in tree::all_nodes() {
        let out = run_inspector(Some(n.id));
        assert!(!out.shapes.is_empty(), "nothing drawn for {}", n.id);
    }
}

#[test]
fn staging_page_has_the_table_and_the_limits() {
    let texts = render_inspector_texts(Some("aligners.planning.staging")).join("\n");
    // nothing here changes the language: assert on structure, not on the wording
    let slider_labels = [
        "aligners-staging-step-linear",
        "aligners-staging-step-vertical",
        "aligners-staging-step-rotation",
        "aligners-staging-step-rotation-round",
        "aligners-staging-step-angular",
        "aligners-staging-max-crown-step",
    ];
    for key in slider_labels
        .iter()
        .chain(&["aligners-staging-stages", "aligners-staging-col-stage"])
    {
        assert!(!dc_i18n::tr(key).is_empty());
    }
    assert!(
        texts.contains("0.25"),
        "default step_linear missing: {texts}"
    );
    assert!(texts.contains("2.00"), "default rotation missing: {texts}");
    assert!(texts.contains("0.33"), "default crown cap missing: {texts}");
    // other pages do not carry the staging layout
    let other = render_inspector_texts(Some("aligners.planning.ipr")).join("\n");
    assert!(!other.contains("0.33"));
}
