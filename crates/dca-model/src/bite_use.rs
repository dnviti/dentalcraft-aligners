//! Whether a case may use a bite: the relation of the upper and the lower
//! arch, and what needs the other arch (the registration on the bite scans
//! and the virtual bite, the contact map, the correction of the
//! interpenetration, the bite with the aligners worn, the occlusal shaping of
//! the aligners, the bite ramps, the Bolton ratios).
//!
//! The user's rule (2026-11-23): a case with only one arch has no bite. It is
//! never estimated, synthesised or taken by default; a bite the case stores
//! (an old project, a project whose other arch was removed) is ignored and
//! not shown; the aligners are made from the one arch alone, and nothing
//! that needs the other arch is in what the stages give. With both arches
//! nothing changes: the bite is registered on the bite scans, or estimated by
//! the virtual bite when the case has none (the plugin's `VIRTUALE`, shown as
//! an estimate).
//!
//! [`bite_use`] is the one decision; every stage asks it, for a case through
//! [`BiteUse::of`].
//!
//! Departures from the plugin, which with one arch also finds no bite
//! (`case_plan.opposing_pair`, `case_occlusal.pair`):
//! * the bite commands answer that the bite does not apply, with the case
//!   unchanged, instead of an error asking for the other arch;
//! * the step "Bite" of a case with one arch has nothing left to do, so it
//!   never stands as the next step of the workflow;
//! * the ramps a case with one arch still stores do not enter its aligners
//!   (the plugin adds the ramps of a case whatever the other arch).

use dc_case::{CaseState, ScanRole};
use serde::Serialize;

use crate::{AlignersState, BiteMethod, MODULE_ID};

/// What a case stores of a bite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StoredBite {
    /// A registration or the virtual bite moved the arches
    /// ([`AlignersState::bite_registered`]).
    pub registered: bool,
    /// How the last bite was found ([`AlignersState::bite_report`]), `None`
    /// without a report.
    pub method: Option<BiteMethod>,
    /// The case holds bite ramps.
    pub ramps: bool,
}

impl StoredBite {
    /// What `data` stores of a bite.
    pub fn of(data: &AlignersState) -> Self {
        Self {
            registered: data.bite_registered,
            method: data.bite_report.as_ref().map(|r| r.method),
            ramps: !data.aligner_bite.ramps.ramps.is_empty(),
        }
    }

    /// Something of a bite is stored.
    pub fn any(self) -> bool {
        self.registered || self.method.is_some() || self.ramps
    }
}

/// What a case may do with a bite ([`bite_use`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BiteUse {
    /// No arch yet: there is nothing to relate.
    NoArch,
    /// One arch: there is no bite. Nothing is estimated, synthesised or taken
    /// by default, and what the case stores of a bite is ignored: the bite
    /// does not apply.
    OneArch,
    /// Both arches and no bite stored: the step "Bite" relates them (the
    /// registration on the bite scans, or the virtual bite without one).
    ToRelate,
    /// Both arches and a bite stored: it is used.
    Stored,
}

impl BiteUse {
    /// The decision for the case `state`: the arches it has a scan of, and
    /// what its Aligners data stores of a bite.
    pub fn of(state: &CaseState) -> Self {
        let stored = state
            .module_data::<AlignersState>(MODULE_ID)
            .map(StoredBite::of)
            .unwrap_or_default();
        bite_use(
            state.case.scan(ScanRole::Upper).is_some(),
            state.case.scan(ScanRole::Lower).is_some(),
            stored,
        )
    }

    /// Both arches: the bite and what needs the other arch apply.
    pub fn applies(self) -> bool {
        matches!(self, Self::ToRelate | Self::Stored)
    }

    /// The bite the case stores may be used and shown.
    pub fn uses_stored(self) -> bool {
        self == Self::Stored
    }

    /// One arch: the bite does not apply.
    pub fn one_arch(self) -> bool {
        self == Self::OneArch
    }
}

/// Whether a case with a scan of the upper arch (`upper`), of the lower arch
/// (`lower`) and storing `stored` may use a bite: with one arch never, with
/// both arches the bite stored, or the one the step "Bite" finds.
pub fn bite_use(upper: bool, lower: bool, stored: StoredBite) -> BiteUse {
    match (upper, lower) {
        (false, false) => BiteUse::NoArch,
        (true, true) if stored.any() => BiteUse::Stored,
        (true, true) => BiteUse::ToRelate,
        _ => BiteUse::OneArch,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BiteReport, Ramp, RampKind};
    use dc_case::{Fdi, ModuleState, Pose, Scan};

    fn every_stored() -> Vec<StoredBite> {
        let methods = [
            None,
            Some(BiteMethod::Registration),
            Some(BiteMethod::ThreePoints),
            Some(BiteMethod::Virtual),
        ];
        let mut all = Vec::new();
        for registered in [false, true] {
            for method in methods {
                for ramps in [false, true] {
                    all.push(StoredBite {
                        registered,
                        method,
                        ramps,
                    });
                }
            }
        }
        all
    }

    #[test]
    fn one_arch_has_no_bite_whatever_the_case_stores() {
        for stored in every_stored() {
            for (upper, lower) in [(true, false), (false, true)] {
                let u = bite_use(upper, lower, stored);
                assert_eq!(u, BiteUse::OneArch, "{stored:?}");
                assert!(!u.applies() && !u.uses_stored() && u.one_arch());
            }
            let none = bite_use(false, false, stored);
            assert_eq!(none, BiteUse::NoArch);
            assert!(!none.applies() && !none.uses_stored() && !none.one_arch());
            let both = bite_use(true, true, stored);
            assert!(both.applies() && !both.one_arch());
            assert_eq!(both.uses_stored(), stored.any(), "{stored:?}");
        }
    }

    fn scan(role: ScanRole, n: u8) -> Scan {
        serde_json::from_value(serde_json::json!({
            "id": format!("00000000-0000-4000-8000-00000000000{n}"),
            "role": role,
            "source_name": format!("scan-{n}.stl"),
            "source_units": "mm",
            "import_xform": { "t": [0.0, 0.0, 0.0], "q": [0.0, 0.0, 0.0, 1.0] },
            "mesh": dc_case::BlobRef::of_bytes(&[n]),
        }))
        .unwrap()
    }

    #[test]
    fn a_case_is_read_from_its_scans_and_its_data() {
        let mut state = CaseState::default();
        assert_eq!(BiteUse::of(&state), BiteUse::NoArch);
        // a stale bite of an old project with the upper arch only: ignored
        let mut data = AlignersState {
            bite_registered: true,
            bite_report: Some(BiteReport::virtual_bite()),
            ..AlignersState::default()
        };
        data.aligner_bite.ramps.ramps.push(Ramp {
            fdi: Fdi::new(11).unwrap(),
            pose: Pose::IDENTITY,
            size_mm: [3.0, 2.0, 1.5],
            kind: RampKind::Manual,
        });
        state.set_module(MODULE_ID, ModuleState::new(data.clone()));
        state.case.scans.push(scan(ScanRole::Upper, 1));
        // a bite scan is no arch
        state.case.scans.push(scan(ScanRole::Bite, 2));
        assert_eq!(BiteUse::of(&state), BiteUse::OneArch);
        state.case.scans.push(scan(ScanRole::Lower, 3));
        assert_eq!(BiteUse::of(&state), BiteUse::Stored);
        state.set_module(MODULE_ID, ModuleState::new(AlignersState::default()));
        assert_eq!(BiteUse::of(&state), BiteUse::ToRelate);
        assert_eq!(StoredBite::of(&data).method, Some(BiteMethod::Virtual));
        assert!(StoredBite::of(&data).registered && StoredBite::of(&data).ramps);
        assert!(!StoredBite::of(&AlignersState::default()).any());
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    fn any_stored() -> StoredBite {
        StoredBite {
            registered: kani::any(),
            method: match kani::any::<u8>() % 4 {
                0 => None,
                1 => Some(BiteMethod::Registration),
                2 => Some(BiteMethod::ThreePoints),
                _ => Some(BiteMethod::Virtual),
            },
            ramps: kani::any(),
        }
    }

    // A case with one arch never uses a bite, whatever it stores of one (a
    // registration, three points, a virtual bite, ramps): the bite does not
    // apply and nothing stored is used.
    #[kani::proof]
    fn one_arch_never_uses_a_bite() {
        let upper: bool = kani::any();
        let lower: bool = kani::any();
        let stored = any_stored();
        let u = bite_use(upper, lower, stored);
        if upper != lower {
            assert!(u.one_arch());
            assert!(!u.applies());
            assert!(!u.uses_stored());
        }
    }

    // No bite is ever taken without both arches and something stored: with
    // no arch or one arch nothing is used, and with both arches and nothing
    // stored no bite is assumed (the step "Bite" finds one).
    #[kani::proof]
    fn a_bite_is_used_only_when_both_arches_store_one() {
        let upper: bool = kani::any();
        let lower: bool = kani::any();
        let stored = any_stored();
        let u = bite_use(upper, lower, stored);
        assert_eq!(u.uses_stored(), upper && lower && stored.any());
    }

    // With both arches the bite applies, whatever is stored: the rule leaves
    // the cases with two arches as they were.
    #[kani::proof]
    fn both_arches_keep_the_bite() {
        let stored = any_stored();
        let u = bite_use(true, true, stored);
        assert!(u.applies());
        assert!(!u.one_arch());
    }
}
