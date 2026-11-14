//! What was changed by hand on the 3D components with the gizmo of the
//! viewport (`aligners.components.transform`): the rigid move of a scan,
//! with everything of its arch, and the size of a built tooth.
//!
//! The move of a tooth is not kept here: it is a pose of the plan (the final
//! position, [`crate::Setup`], or an aligner, [`crate::Stage`]), as the
//! manual adjustment of a tooth stores it.

use std::collections::BTreeMap;

use dc_case::{BlobRef, Fdi, Pose};
use glam::{DVec3, Vec3};
use serde::{Deserialize, Serialize};

use crate::AlignersState;

/// Smallest size factor of a built tooth on the size it was built with.
pub const TOOTH_SCALE_MIN: f64 = 0.5;
/// Largest size factor of a built tooth on the size it was built with.
pub const TOOTH_SCALE_MAX: f64 = 2.0;

/// The centre a built tooth is scaled about: the mean of the points of its
/// closed solid. It moves with the tooth, and a tooth scaled about it keeps
/// it, so the viewport's gizmo and the scale agree whatever the tooth went
/// through.
pub fn tooth_centre(positions: &[Vec3]) -> DVec3 {
    let n = positions.len().max(1) as f64;
    positions.iter().map(|p| p.as_dvec3()).sum::<DVec3>() / n
}

/// The size of a built tooth corrected by hand.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ToothScale {
    /// Factor on the size the tooth was built with (`1` = as built), about
    /// the centre of the tooth (the mean of its points).
    pub factor: f64,
    /// The closed solid the factor gave. The factor holds while the tooth
    /// has this solid; a solid moved with its arch keeps its factor
    /// ([`ComponentEdits::crown_moved`]); a tooth built again has its built
    /// size.
    pub crown: BlobRef,
}

/// The changes made by hand to the components of the case.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ComponentEdits {
    /// The rigid move made by hand to each scan, keyed by the scan id, in
    /// case coordinates (mm): where the scan is now from where it was before
    /// the first move by hand. Moving the scan of an arch moved everything of
    /// that arch with it (its tooth colours and marks, its 3D teeth and
    /// gingiva); a bite scan moves alone. A scan with no entry was not moved.
    pub scan_moves: BTreeMap<String, Pose>,
    /// The built teeth whose size was corrected by hand.
    pub tooth_scales: BTreeMap<Fdi, ToothScale>,
}

impl ComponentEdits {
    /// The move made by hand to the scan `id` (the identity when none).
    pub fn scan_move(&self, id: &str) -> Pose {
        self.scan_moves.get(id).copied().unwrap_or(Pose::IDENTITY)
    }

    /// Records `pose` as the move made by hand to the scan `id`; the
    /// identity removes the entry.
    pub fn set_scan_move(&mut self, id: &str, pose: Pose) {
        if pose.approx_eq(&Pose::IDENTITY, 1e-9, 1e-9) {
            self.scan_moves.remove(id);
        } else {
            self.scan_moves.insert(id.to_owned(), pose);
        }
    }

    /// The closed solid `old` was moved rigidly and is now `new`: a size
    /// corrected by hand on it holds on the moved solid.
    pub fn crown_moved(&mut self, old: BlobRef, new: BlobRef) {
        for s in self.tooth_scales.values_mut() {
            if s.crown == old {
                s.crown = new;
            }
        }
    }
}

impl AlignersState {
    /// The size factor of the built tooth `f` on the size it was built with:
    /// its correction by hand while the tooth has the solid that correction
    /// gave, else `1` (no built tooth, no correction, or the tooth was built
    /// again since).
    pub fn tooth_scale(&self, f: Fdi) -> f64 {
        match (self.components.tooth_scales.get(&f), self.crowns.get(&f)) {
            (Some(s), Some(crown)) if s.crown == *crown => s.factor,
            _ => 1.0,
        }
    }

    /// Drops the corrections by hand that no longer describe the case: the
    /// moves of scans the case does not hold (`scan_ids`: the ids of its
    /// scans) and the sizes of teeth that were built again or removed.
    pub fn prune_component_edits<'a>(&mut self, scan_ids: impl IntoIterator<Item = &'a str>) {
        let ids: Vec<&str> = scan_ids.into_iter().collect();
        self.components
            .scan_moves
            .retain(|id, _| ids.contains(&id.as_str()));
        let crowns = &self.crowns;
        self.components
            .tooth_scales
            .retain(|f, s| crowns.get(f) == Some(&s.crown));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::{DQuat, DVec3};

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    #[test]
    fn a_scale_holds_while_the_tooth_has_the_solid_it_gave() {
        let mut st = AlignersState::default();
        let built = BlobRef::of_bytes(b"built");
        let scaled = BlobRef::of_bytes(b"scaled");
        st.crowns.insert(fdi(36), scaled);
        assert_eq!(st.tooth_scale(fdi(36)), 1.0);
        st.components.tooth_scales.insert(
            fdi(36),
            ToothScale {
                factor: 1.1,
                crown: scaled,
            },
        );
        assert_eq!(st.tooth_scale(fdi(36)), 1.1);
        // moved with its arch: the factor follows the moved solid
        let moved = BlobRef::of_bytes(b"moved");
        st.crowns.insert(fdi(36), moved);
        st.components.crown_moved(scaled, moved);
        assert_eq!(st.tooth_scale(fdi(36)), 1.1);
        // built again: the built size, and the old entry goes when pruned
        st.crowns.insert(fdi(36), built);
        assert_eq!(st.tooth_scale(fdi(36)), 1.0);
        st.prune_component_edits([]);
        assert!(st.components.tooth_scales.is_empty());
    }

    #[test]
    fn scan_moves_are_kept_by_scan_and_the_identity_is_no_move() {
        let mut e = ComponentEdits::default();
        let p = Pose::new(DVec3::new(1.0, 0.0, 0.0), DQuat::from_rotation_z(0.1));
        e.set_scan_move("a", p);
        assert_eq!(e.scan_move("a"), p);
        assert_eq!(e.scan_move("b"), Pose::IDENTITY);
        e.set_scan_move("a", Pose::IDENTITY);
        assert!(e.scan_moves.is_empty());
        e.set_scan_move("a", p);
        e.set_scan_move("gone", p);
        let mut st = AlignersState {
            components: e,
            ..AlignersState::default()
        };
        st.prune_component_edits(["a"]);
        assert_eq!(st.components.scan_moves.keys().collect::<Vec<_>>(), ["a"]);
    }

    #[test]
    fn the_edits_survive_json_and_older_files_have_none() {
        let mut st = AlignersState::default();
        st.components
            .set_scan_move("s", Pose::from_translation(DVec3::new(0.5, 1.0, -2.0)));
        st.components.tooth_scales.insert(
            fdi(11),
            ToothScale {
                factor: 0.9,
                crown: BlobRef::of_bytes(b"c"),
            },
        );
        let v = serde_json::to_value(&st).unwrap();
        let back: AlignersState = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(back.components, st.components);
        // a file written before the gizmo has no `components`
        let mut old = v;
        old.as_object_mut().unwrap().remove("components");
        let back: AlignersState = serde_json::from_value(old).unwrap();
        assert_eq!(back.components, ComponentEdits::default());
    }
}
