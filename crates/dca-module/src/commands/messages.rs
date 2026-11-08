//! The sentences of the errors the commands report when a step they depend
//! on has not been done (no scans, no segmentation, no labels), in the
//! language of the app (Fluent keys `aligners-err-*`).

use dc_command::CommandError;
use dca_algo::anatomy::Arch;
use dca_algo::autoseed::AutoseedError;
use dca_algo::frame::FrameError;
use dca_algo::segmentation::{MeshFault, MeshPart, SegmentationError};

/// `key` with its arguments.
pub(crate) fn tr(key: &str, args: &[(&str, &str)]) -> String {
    crate::loader();
    dc_i18n::tr_args(key, args)
}

/// [`CommandError::Invalid`] with the sentence of `key`.
pub(crate) fn invalid(key: &str, args: &[(&str, &str)]) -> CommandError {
    CommandError::Invalid(tr(key, args))
}

/// The name of an arch inside a sentence (`upper`, `superiore`).
pub(crate) fn arch_word(arch: Arch) -> String {
    tr(
        if arch.is_upper() {
            "aligners-err-arch-upper"
        } else {
            "aligners-err-arch-lower"
        },
        &[],
    )
}

/// The Aligners data of the case cannot be read.
pub(crate) fn no_data() -> CommandError {
    invalid("aligners-err-no-data", &[])
}

/// The case holds no Aligners data.
pub(crate) fn no_case_data() -> CommandError {
    invalid("aligners-err-no-case-data", &[])
}

/// The arch has no segmented scan.
pub(crate) fn not_segmented(arch: Arch) -> CommandError {
    invalid(
        "aligners-err-arch-not-segmented",
        &[("arch", &arch_word(arch))],
    )
}

/// No arch has a segmented scan.
pub(crate) fn none_segmented() -> CommandError {
    invalid("aligners-err-none-segmented", &[])
}

/// The labels of the arch are missing.
pub(crate) fn labels_missing(arch: Arch) -> CommandError {
    invalid("aligners-err-labels-missing", &[("arch", &arch_word(arch))])
}

/// The tooth labels are missing (the arch is not named).
pub(crate) fn tooth_labels_missing() -> CommandError {
    invalid("aligners-err-tooth-labels-missing", &[])
}

/// The labels of the arch do not fit the scan.
pub(crate) fn labels_mismatch(arch: Arch, labels: usize, vertices: usize) -> CommandError {
    invalid(
        "aligners-err-labels-mismatch",
        &[
            ("arch", &arch_word(arch)),
            ("labels", &labels.to_string()),
            ("vertices", &vertices.to_string()),
        ],
    )
}

/// The labels do not fit the scan (the arch is not named).
pub(crate) fn tooth_labels_mismatch(labels: usize, vertices: usize) -> CommandError {
    invalid(
        "aligners-err-tooth-labels-mismatch",
        &[
            ("labels", &labels.to_string()),
            ("vertices", &vertices.to_string()),
        ],
    )
}

/// The case has no scan for the arch.
pub(crate) fn no_scan(arch: Arch) -> CommandError {
    invalid("aligners-err-no-scan", &[("arch", &arch_word(arch))])
}

/// The case has neither an upper nor a lower scan.
pub(crate) fn no_scan_any() -> CommandError {
    invalid("aligners-err-no-scan-any", &[])
}

/// The scan of the arch is not there (it was there a moment ago).
pub(crate) fn scan_missing(arch: Arch) -> CommandError {
    invalid("aligners-err-scan-missing", &[("arch", &arch_word(arch))])
}

/// The scan the seeds are proposed from is not there.
pub(crate) fn reference_missing() -> CommandError {
    invalid("aligners-err-reference-missing", &[])
}

/// The arch has no tooth seeds.
pub(crate) fn no_seeds(arch: Arch) -> CommandError {
    invalid("aligners-err-arch-no-seeds", &[("arch", &arch_word(arch))])
}

/// No arch has a scan with tooth seeds.
pub(crate) fn none_seeded() -> CommandError {
    invalid("aligners-err-none-seeded", &[])
}

/// The sentence for an error of the automatic seeds on the scan of `arch`
/// (`None` when the error is about the parameters and not about a scan). The
/// ones about the data say to mark the teeth by hand.
pub(crate) fn autoseed_error(arch: Option<Arch>, e: &AutoseedError) -> CommandError {
    let arch = arch.map(arch_word).unwrap_or_default();
    let arch = [("arch", arch.as_str())];
    match e {
        AutoseedError::BadEnds => invalid("aligners-err-seed-bad-ends", &[]),
        AutoseedError::BadTooth(n) => {
            invalid("aligners-err-seed-bad-tooth", &[("fdi", &n.to_string())])
        }
        AutoseedError::DegenerateMesh => {
            CommandError::Failed(tr("aligners-err-seed-degenerate", &arch))
        }
        AutoseedError::ArchNotFound => {
            CommandError::Failed(tr("aligners-err-seed-arch-not-found", &arch))
        }
        AutoseedError::TooFewSeeds(n) => CommandError::Failed(tr(
            "aligners-err-seed-too-few",
            &[("arch", arch[0].1), ("found", &n.to_string())],
        )),
    }
}

/// The sentence for an error of the frame stage on the scan of `arch`. The
/// ones about the seeds say to place them by hand or let the program find them.
pub(crate) fn frame_error(arch: Arch, e: &FrameError) -> CommandError {
    let arch_w = arch_word(arch);
    let arch = [("arch", arch_w.as_str())];
    match e {
        FrameError::TooFewSeeds => invalid("aligners-err-frame-too-few-seeds", &arch),
        FrameError::SeedsOnOneSide => invalid("aligners-err-frame-one-side", &arch),
        FrameError::DegenerateMesh => {
            CommandError::Failed(tr("aligners-err-frame-degenerate", &arch))
        }
        FrameError::NoArchScan => no_scan_any(),
    }
}

/// The sentence for an error of the segmentation of `arch`.
pub(crate) fn segmentation_error(arch: Arch, e: &SegmentationError) -> CommandError {
    let arch_w = arch_word(arch);
    let arch_a = [("arch", arch_w.as_str())];
    match e {
        SegmentationError::BadMesh(part, fault) => {
            let part = tr(
                match part {
                    MeshPart::Scan => "aligners-err-seg-part-scan",
                    MeshPart::Simplified => "aligners-err-seg-part-simplified",
                },
                &[],
            );
            let fault = tr(
                match fault {
                    MeshFault::NoTriangles => "aligners-err-seg-fault-no-triangles",
                    MeshFault::IndexOutOfRange => "aligners-err-seg-fault-index",
                    MeshFault::NotFinite => "aligners-err-seg-fault-not-finite",
                },
                &[],
            );
            invalid(
                "aligners-err-seg-bad-mesh",
                &[("part", &part), ("arch", &arch_w), ("fault", &fault)],
            )
        }
        SegmentationError::SeedNotFinite => invalid("aligners-err-seg-seed-not-finite", &arch_a),
        SegmentationError::NoSeeds => no_seeds(arch),
        SegmentationError::Decimate(detail) => CommandError::Failed(tr(
            "aligners-err-seg-decimate",
            &[("arch", &arch_w), ("detail", detail)],
        )),
        SegmentationError::Transfer(detail) => CommandError::Failed(tr(
            "aligners-err-seg-transfer",
            &[("arch", &arch_w), ("detail", detail)],
        )),
        SegmentationError::TargetFacesRange(_)
        | SegmentationError::EtaConvexRange(_)
        | SegmentationError::SigmaRange(_) => segmentation_params_error(e),
        SegmentationError::Cancelled => CommandError::Cancelled,
    }
}

/// Doubts of one arch shown in a warning, at most (the plugin shows six).
const DOUBTS_SHOWN: usize = 6;

/// The warning for the doubts the check of the separated teeth left on
/// `arch` (`AlignersState::segmentation_doubts`), as the plugin's export and
/// report give it: the aligners must not be delivered before the tooth
/// marks and their numbers are checked.
pub(crate) fn segmentation_doubts_warning(arch: Arch, doubts: &[String]) -> String {
    let list: Vec<&str> = doubts
        .iter()
        .take(DOUBTS_SHOWN)
        .map(String::as_str)
        .collect();
    tr(
        "aligners-warn-segmentation-doubts",
        &[("arch", &arch_word(arch)), ("list", &list.join("; "))],
    )
}

/// The doubts of the case by arch (`upper`, `lower`) and the warnings about
/// them, upper arch first: the `segmentation_doubts` and the entries for
/// `warnings` of the outputs of the export, the report and `aligners.auto`.
pub(crate) fn segmentation_doubts(
    data: &dca_model::AlignersState,
) -> (serde_json::Value, Vec<String>) {
    let mut by_arch = serde_json::Map::new();
    let mut warnings = Vec::new();
    for arch in [Arch::Upper, Arch::Lower] {
        if let Some(d) = data
            .segmentation_doubts
            .get(&arch.role())
            .filter(|d| !d.is_empty())
        {
            let name = if arch.is_upper() { "upper" } else { "lower" };
            by_arch.insert(name.to_owned(), serde_json::json!(d));
            warnings.push(segmentation_doubts_warning(arch, d));
        }
    }
    (serde_json::Value::Object(by_arch), warnings)
}

/// The sentence for a parameter of the segmentation outside its range (any
/// other error of the segmentation is about a scan, see
/// [`segmentation_error`]).
pub(crate) fn segmentation_params_error(e: &SegmentationError) -> CommandError {
    match e {
        SegmentationError::TargetFacesRange(v) => invalid(
            "aligners-err-seg-target-faces",
            &[("value", &v.to_string())],
        ),
        SegmentationError::EtaConvexRange(v) => {
            invalid("aligners-err-seg-eta-convex", &[("value", &v.to_string())])
        }
        SegmentationError::SigmaRange(v) => {
            invalid("aligners-err-seg-sigma", &[("value", &v.to_string())])
        }
        other => CommandError::Failed(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(e: CommandError) -> String {
        match e {
            CommandError::Invalid(m) | CommandError::Failed(m) => m,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn every_frame_and_segmentation_error_is_a_translated_sentence() {
        let mut all = Vec::new();
        for e in [
            FrameError::TooFewSeeds,
            FrameError::SeedsOnOneSide,
            FrameError::DegenerateMesh,
        ] {
            all.push(text(frame_error(Arch::Lower, &e)));
        }
        let parts = [MeshPart::Scan, MeshPart::Simplified];
        let faults = [
            MeshFault::NoTriangles,
            MeshFault::IndexOutOfRange,
            MeshFault::NotFinite,
        ];
        for part in parts {
            for fault in faults {
                all.push(text(segmentation_error(
                    Arch::Upper,
                    &SegmentationError::BadMesh(part, fault),
                )));
            }
        }
        for e in [
            SegmentationError::SeedNotFinite,
            SegmentationError::NoSeeds,
            SegmentationError::Decimate("x".to_owned()),
            SegmentationError::Transfer("x".to_owned()),
            SegmentationError::TargetFacesRange(1),
            SegmentationError::EtaConvexRange(2.0),
            SegmentationError::SigmaRange(0.0),
        ] {
            all.push(text(segmentation_error(Arch::Upper, &e)));
        }
        for m in &all {
            // a key that is missing from the files comes back as itself
            assert!(!m.contains("aligners-"), "{m}");
            assert!(!m.contains('{') && !m.contains("$arch"), "{m}");
        }
        assert!(all[0].contains("lower arch"), "{}", all[0]);
        assert!(all[3].contains("upper arch"), "{}", all[3]);
        assert!(
            text(segmentation_params_error(
                &SegmentationError::TargetFacesRange(7)
            ))
            .contains("target_faces")
        );
        assert!(matches!(
            segmentation_error(Arch::Upper, &SegmentationError::Cancelled),
            CommandError::Cancelled
        ));
        let many: Vec<String> = (1..=8).map(|k| format!("doubt {k}")).collect();
        let w = segmentation_doubts_warning(Arch::Lower, &many);
        assert!(w.contains("lower arch") && w.contains("doubt 6"), "{w}");
        assert!(!w.contains("doubt 7") && !w.contains("aligners-"), "{w}");
    }
}
