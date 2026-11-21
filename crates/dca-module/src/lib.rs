//! The Aligners module for DentalCraft (id `aligners`, shown as
//! "Aligners" / "Allineatori").
//!
//! [`AlignersModule`] implements [`dc_module_api::Module`]. The menu
//! ([`tree`]) holds what the end user needs; every other command of the
//! registry ([`commands`]) is reachable from the CLI and the MCP server. The
//! algorithms behind the entries live in `dca-algo`, the data in `dca-model`.

mod aligner_scene;
mod codec;
pub mod commands;
mod done;
mod find_teeth;
mod inspector;
mod manual;
mod open_scan;
mod print_ui;
mod scene;
pub mod tree;
mod tree_data;

use dc_case::{CaseState, ModuleState};
use dc_module_api::{
    CommandDesc, CommandSpec, MenuStatus, MenuTree, Module, ModuleCodec, ModuleCtx, ModuleError,
    ModuleRegistry, NOT_AVAILABLE_KEY, Toast, ToastLevel, is_import_scans_command,
};
use dc_scene::{SceneEntry, SceneItem};
use dca_model::AlignersState;

pub use aligner_scene::ALIGNER_COLOR;
pub use dca_model::MODULE_ID;
pub use scene::{GINGIVA_COLOR, STUDY_MODEL_COLOR, is_study_model, study_models_drawn};

dc_i18n::bundle!(pub fn loader);

/// i18n key of the module's display name.
pub const NAME_KEY: &str = "aligners-name";

/// The Aligners module.
#[derive(Debug)]
pub struct AlignersModule {
    selected: Option<&'static str>,
    /// Study base settings being edited in the inspector.
    study_ui: inspector::StudyEdit,
    /// The field of the missing teeth being edited in the inspector.
    find_ui: find_teeth::FindEdit,
    /// Blend weights of the built gingiva, computed when a stage first
    /// needs them.
    gums: scene::GumCache,
    /// The aligners of the 3D view bound to the model, computed when a
    /// stage first shows one away from the stage it was built at.
    binds: aligner_scene::BindCache,
}

impl Default for AlignersModule {
    fn default() -> Self {
        Self::new()
    }
}

impl AlignersModule {
    /// New module with nothing selected.
    pub fn new() -> Self {
        loader(); // registers the translations
        Self {
            selected: None,
            study_ui: None,
            find_ui: find_teeth::FindEdit::default(),
            gums: scene::GumCache::default(),
            binds: aligner_scene::BindCache::default(),
        }
    }

    /// Shows menu entry `id` (a command, group or sub-menu) in the
    /// inspector. Returns `false` and changes nothing for an unknown id.
    pub fn select_entry(&mut self, id: &str) -> bool {
        match tree::find(id) {
            Some(node) => {
                self.selected = Some(node.id);
                true
            }
            None => false,
        }
    }

    /// The entry shown in the inspector.
    pub fn selected(&self) -> Option<&'static str> {
        self.selected
    }

    /// Empty module data for a new case.
    pub fn initial_state() -> ModuleState {
        ModuleState::new(AlignersState::default())
    }
}

impl Module for AlignersModule {
    fn id(&self) -> &'static str {
        MODULE_ID
    }

    fn name_key(&self) -> &'static str {
        NAME_KEY
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn menu(&self) -> MenuTree {
        tree::menu_tree()
    }

    fn commands(&self) -> Vec<CommandDesc> {
        tree::command_descs()
    }

    fn command_specs(&self) -> Vec<CommandSpec> {
        commands::all()
    }

    fn invoke(&mut self, command_id: &str, ctx: &mut ModuleCtx<'_>) -> Result<(), ModuleError> {
        let node = tree::find(command_id)
            .filter(|n| n.is_leaf())
            .ok_or_else(|| ModuleError::UnknownCommand(command_id.to_owned()))?;
        self.selected = Some(node.id);
        match node.status() {
            MenuStatus::Stub => {
                ctx.toast(Toast::new(ToastLevel::Info, NOT_AVAILABLE_KEY));
                Ok(())
            }
            // the import needs its files, a leaf with an input its answer: the
            // shell asks for them and runs the command
            MenuStatus::Ready if is_import_scans_command(node.id) || node.input.is_some() => Ok(()),
            // a Ready leaf is a registered command: run it with its defaults
            MenuStatus::Ready => {
                let spec = commands::find(node.id)
                    .ok_or_else(|| ModuleError::UnknownCommand(command_id.to_owned()))?;
                // the options of Find the teeth are in the inspector
                let params = if node.id == commands::wrappers::FIND_TEETH {
                    find_teeth::leaf_params(ctx.state())
                } else {
                    serde_json::Value::Null
                };
                ctx.queue(Box::new(spec.command(params)));
                Ok(())
            }
        }
    }

    fn leaf_done(&self, state: &CaseState, leaf_id: &str) -> bool {
        done::leaf_done(state, leaf_id)
    }

    fn select(&mut self, id: &str) {
        self.select_entry(id);
    }

    /// A group, or nothing selected: its title and description (the
    /// overview). A leaf: only its parameters, if it has any (Find the teeth,
    /// Study models, the tracking of Print); the shell lays out the rest of
    /// the inspector of a leaf (title, description, status, run button and
    /// manual tools) and draws this in its section "Parameters".
    fn inspector(&mut self, ui: &mut egui::Ui, ctx: &mut ModuleCtx<'_>) {
        loader();
        match self.selected.and_then(tree::find).filter(|n| n.is_leaf()) {
            Some(n) if n.id == commands::study_base::STUDY_MODELS_ID => {
                inspector::study_settings(ui, ctx, &mut self.study_ui);
            }
            Some(n) if n.id == commands::wrappers::FIND_TEETH => {
                find_teeth::show(ui, ctx, &mut self.find_ui);
            }
            Some(n) if n.id == commands::print::PRINT => print_ui::show(ui, ctx),
            Some(_) => {}
            None => inspector::show(ui, self.selected),
        }
    }

    fn scene(&self, _state: &CaseState) -> Vec<SceneItem> {
        Vec::new()
    }

    /// The built teeth and gingiva, the study models, and the aligner of
    /// the 3D view while the case shows it (`aligner_scene`).
    fn scene_at(&self, state: &CaseState, stage: Option<u32>) -> Vec<SceneEntry> {
        let mut out = scene::built_scene(state, stage, &self.gums, true).0;
        out.extend(aligner_scene::entries(state, stage, &self.gums, &self.binds, true).0);
        out
    }

    fn scene_at_nowait(&self, state: &CaseState, stage: Option<u32>) -> (Vec<SceneEntry>, bool) {
        let (mut out, done) = scene::built_scene(state, stage, &self.gums, false);
        let (aligner, ready) = aligner_scene::entries(state, stage, &self.gums, &self.binds, false);
        out.extend(aligner);
        (out, done && ready)
    }

    fn data_codec(&self) -> ModuleCodec {
        codec::codec()
    }
}

/// Registers the Aligners module in `registry`.
pub fn register(registry: &mut ModuleRegistry) -> Result<(), ModuleError> {
    registry.register(Box::new(AlignersModule::new()))
}
