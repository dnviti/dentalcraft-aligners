//! The Aligners module for DentalCraft (id `aligners`, shown as
//! "Aligners" / "Allineatori").
//!
//! [`AlignersModule`] implements [`dc_module_api::Module`]. The whole menu
//! ([`tree`]) is there; an entry is implemented when its command is in the
//! registry ([`commands`]), otherwise it is a placeholder: invoking it
//! selects the entry in the inspector and shows the "not available yet"
//! toast. The algorithms behind the entries live in `dca-algo`, the data in
//! `dca-model`.

mod codec;
pub mod commands;
mod inspector;
mod manual;
pub mod tree;
mod tree_data;

use dc_case::{CaseState, ModuleState};
use dc_module_api::{
    CommandDesc, CommandSpec, MenuStatus, MenuTree, Module, ModuleCodec, ModuleCtx, ModuleError,
    ModuleRegistry, NOT_AVAILABLE_KEY, Toast, ToastLevel, is_import_scans_command,
};
use dc_scene::SceneItem;
use dca_model::{AlignersState, StagingSettings};

pub use dca_model::MODULE_ID;

dc_i18n::bundle!(pub fn loader);

/// i18n key of the module's display name.
pub const NAME_KEY: &str = "aligners-name";

/// The Aligners module.
#[derive(Debug)]
pub struct AlignersModule {
    selected: Option<&'static str>,
    /// Values of the staging limit sliders (placeholder page).
    staging_ui: StagingSettings,
    /// Study base settings being edited in the inspector.
    study_ui: inspector::StudyEdit,
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
            staging_ui: StagingSettings::default(),
            study_ui: None,
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
            // the import needs its files: the shell asks for them and runs it
            MenuStatus::Ready if is_import_scans_command(node.id) => Ok(()),
            // a Ready leaf is a registered command: run it with its defaults
            MenuStatus::Ready => {
                let spec = commands::find(node.id)
                    .ok_or_else(|| ModuleError::UnknownCommand(command_id.to_owned()))?;
                ctx.queue(Box::new(spec.command(serde_json::Value::Null)));
                Ok(())
            }
        }
    }

    fn select(&mut self, id: &str) {
        self.select_entry(id);
    }

    fn inspector(&mut self, ui: &mut egui::Ui, ctx: &mut ModuleCtx<'_>) {
        loader();
        inspector::show(ui, self.selected, &mut self.staging_ui);
        if inspector::is_study_models(self.selected) {
            inspector::study_settings(ui, ctx, &mut self.study_ui);
        }
    }

    fn scene(&self, _state: &CaseState) -> Vec<SceneItem> {
        Vec::new()
    }

    fn data_codec(&self) -> ModuleCodec {
        codec::codec()
    }
}

/// Registers the Aligners module in `registry`.
pub fn register(registry: &mut ModuleRegistry) -> Result<(), ModuleError> {
    registry.register(Box::new(AlignersModule::new()))
}
