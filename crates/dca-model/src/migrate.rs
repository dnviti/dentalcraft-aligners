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

/// One migration step: turns `value`, of schema `from`, into schema
/// `from + 1`; the current version passes through. The codec of the module
/// chains the steps from an older version up to [`SCHEMA_VERSION`].
///
/// - 1 → 2: the print tracking (`prints`) and the last export folder
///   (`export_dir`) were added. Both are optional with an empty default, so
///   a version 1 state reads as it is: no aligner printed, no export known.
///
/// Keep a fixture of every old version in the tests.
pub fn migrate(value: Value, from: u32) -> Result<Value, ModelError> {
    match from {
        1 | SCHEMA_VERSION => Ok(value),
        other => Err(ModelError::NoMigration(other)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AlignersState;

    #[test]
    fn current_version_passes_through() {
        let v = serde_json::json!({"stages": []});
        assert_eq!(migrate(v.clone(), SCHEMA_VERSION).unwrap(), v);
    }

    #[test]
    fn a_version_1_state_opens_with_no_prints() {
        // a version 1 state as saved before the tracking
        let v1 = serde_json::json!({
            "seeds": [],
            "missing": [11],
            "settings": { "quality": "high" }
        });
        let v2 = migrate(v1, 1).unwrap();
        let s: AlignersState = serde_json::from_value(v2).unwrap();
        assert!(s.prints.is_empty());
        assert!(s.export_dir.is_none());
        assert_eq!(s.missing.len(), 1);
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
