//! Codec of the module data for `.dcprj` files.

use dc_module_api::ModuleCodec;

use dca_model::{AlignersState, SCHEMA_VERSION};

/// Codec of [`AlignersState`]: decodes the current schema and chains the
/// migrations of [`dca_model::migrate`] (one version per step) from older
/// ones.
pub fn codec() -> ModuleCodec {
    (0..SCHEMA_VERSION).fold(
        ModuleCodec::typed::<AlignersState>(SCHEMA_VERSION),
        |c, from| {
            c.with_migration(from, move |v| {
                *v = dca_model::migrate(v.take(), from).map_err(|e| e.to_string())?;
                Ok(())
            })
        },
    )
}
