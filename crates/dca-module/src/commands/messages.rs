//! The sentences of the errors the commands report when a step they depend
//! on has not been done (no scans, no segmentation, no labels), in the
//! language of the app (Fluent keys `aligners-err-*`).

use dc_command::CommandError;
use dca_algo::anatomy::Arch;

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
