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
/// - 2 → 3: the recess between the teeth (`trim.recess_mm`) takes effect:
///   the shell of the smooth border is laid on the interproximal block-out,
///   its faces that far inside the tangents to the crowns. In version 2 the
///   value did nothing at all, so no stored value is a tuned result: every
///   version 2 state gets the default recess
///   ([`crate::TrimParams::default`]) whatever it stored. The aligners built
///   for the 3D view (`aligner_view.bases`) were built without the block-out:
///   they go, and are built again on demand. Nothing else changes.
///
/// Keep a fixture of every old version in the tests.
pub fn migrate(mut value: Value, from: u32) -> Result<Value, ModelError> {
    match from {
        1 | SCHEMA_VERSION => Ok(value),
        2 => {
            if let Some(trim) = value.get_mut("trim").and_then(Value::as_object_mut) {
                trim.insert(
                    "recess_mm".to_owned(),
                    Value::from(crate::TrimParams::default().recess_mm),
                );
            }
            if let Some(view) = value.get_mut("aligner_view").and_then(Value::as_object_mut)
                && view.contains_key("bases")
            {
                view.insert("bases".to_owned(), Value::Array(Vec::new()));
            }
            Ok(value)
        }
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

    /// A synthetic version 2 state: a recess that did nothing, an aligner of
    /// the 3D view built without the block-out, and other data.
    fn version_2(recess: f64) -> Value {
        let mut s = AlignersState::default();
        s.missing.insert(dc_case::Fdi::new(11).unwrap());
        s.trim.recess_mm = recess;
        s.trim.margin_gap_mm = 1.2;
        s.settings.gum_clearance_mm = 0.4;
        s.aligner_view.shown = true;
        s.aligner_view.bases.push(crate::AlignerBase {
            arch: dc_case::ScanRole::Upper,
            stage: 2,
            mesh: dc_case::BlobRef::of_bytes(b"old shell"),
            exact: true,
            voxel_mm: 0.1,
            tolerance_mm: 0.01,
            key: "k".to_owned(),
            seconds: 3.0,
        });
        serde_json::to_value(&s).unwrap()
    }

    /// `v` without the two values the migration to version 3 changes.
    fn rest(mut v: Value) -> Value {
        v["trim"].as_object_mut().unwrap().remove("recess_mm");
        v["aligner_view"].as_object_mut().unwrap().remove("bases");
        v
    }

    #[test]
    fn every_version_2_recess_becomes_the_default_and_the_old_shells_go() {
        let default = crate::TrimParams::default().recess_mm;
        for stored in [0.6, 1.6, 2.5, default] {
            let v2 = version_2(stored);
            let v3 = migrate(v2.clone(), 2).unwrap();
            assert_eq!(v3["trim"]["recess_mm"], Value::from(default), "{stored}");
            assert_eq!(v3["aligner_view"]["bases"], Value::Array(Vec::new()));
            // everything else as it was
            assert_eq!(
                serde_json::to_vec(&rest(v3.clone())).unwrap(),
                serde_json::to_vec(&rest(v2)).unwrap()
            );
            let s: AlignersState = serde_json::from_value(v3).unwrap();
            assert_eq!(s.trim.recess_mm, default);
            assert!(s.aligner_view.bases.is_empty() && s.aligner_view.shown);
        }
    }

    #[test]
    fn a_version_3_state_round_trips() {
        let mut s = AlignersState::default();
        s.trim.recess_mm = 2.0;
        let v = serde_json::to_value(&s).unwrap();
        let back: AlignersState =
            serde_json::from_value(migrate(v, SCHEMA_VERSION).unwrap()).unwrap();
        assert_eq!(back, s);
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
