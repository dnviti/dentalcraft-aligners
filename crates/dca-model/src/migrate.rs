//! Schema migrations of the module data.

use serde_json::Value;

use crate::SCHEMA_VERSION;

/// Errors of the data model.
#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    /// There is no migration path from this schema version.
    #[error("no migration from schema version {0}")]
    NoMigration(u32),
}

/// Upgrades `value` from schema `from` to [`SCHEMA_VERSION`].
///
/// Schema 1 is the first and current version, so for now this only accepts
/// it. When version 2 appears, add a step per version here (each step turns
/// version `n` into `n + 1`) and keep a fixture of every old version in the
/// tests.
pub fn migrate(value: Value, from: u32) -> Result<Value, ModelError> {
    match from {
        SCHEMA_VERSION => Ok(value),
        other => Err(ModelError::NoMigration(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_version_passes_through() {
        let v = serde_json::json!({"stages": []});
        assert_eq!(migrate(v.clone(), SCHEMA_VERSION).unwrap(), v);
    }

    #[test]
    fn unknown_versions_are_refused() {
        assert!(matches!(
            migrate(Value::Null, 0),
            Err(ModelError::NoMigration(0))
        ));
        assert!(migrate(Value::Null, SCHEMA_VERSION + 1).is_err());
    }
}
