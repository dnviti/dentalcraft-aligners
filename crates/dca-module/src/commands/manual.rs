//! Commands of the manual tools (see [`crate::manual`]).

use dc_module_api::CommandSpec;

/// The registered commands of the manual tools, built with `stage_command!`.
pub(crate) fn specs() -> Vec<CommandSpec> {
    vec![
        stage_command! {
            id: "aligners.preparation.segmentation.manual-brush",
            label: "aligners-menu-preparation-segmentation-manual-brush",
            doc: "aligners-cmd-manual-brush-doc",
            params: crate::manual::brush::BrushParams,
            run: crate::manual::brush::run_brush,
        },
        stage_command! {
            id: "aligners.preparation.segmentation.clean-borders",
            label: "aligners-menu-preparation-segmentation-clean-borders",
            doc: "aligners-cmd-manual-clean-doc",
            params: crate::manual::brush::CleanParams,
            run: crate::manual::brush::run_clean,
        },
        stage_command! {
            id: "aligners.preparation.numbering.renumber",
            label: "aligners-menu-preparation-numbering-renumber",
            doc: "aligners-cmd-manual-renumber-doc",
            params: crate::manual::numbering::RenumberParams,
            run: crate::manual::numbering::run_renumber,
        },
        stage_command! {
            id: "aligners.preparation.numbering.mark-missing",
            label: "aligners-menu-preparation-numbering-mark-missing",
            doc: "aligners-cmd-manual-missing-doc",
            params: crate::manual::numbering::MissingParams,
            run: crate::manual::numbering::run_missing,
        },
        stage_command! {
            id: "aligners.preparation.numbering.place-seeds",
            label: "aligners-menu-preparation-numbering-place-seeds",
            doc: "aligners-cmd-manual-seeds-doc",
            params: crate::manual::seeds::SeedParams,
            run: crate::manual::seeds::run_seed,
        },
        stage_command! {
            id: "aligners.manual.rebuild-tooth",
            label: "aligners-cmd-manual-rebuild-tooth",
            doc: "aligners-cmd-manual-rebuild-tooth-doc",
            params: crate::manual::rebuild::RebuildParams,
            run: crate::manual::rebuild::run_rebuild,
        },
        stage_command! {
            id: "aligners.planning.movements",
            label: "aligners-menu-planning-movements",
            doc: "aligners-cmd-manual-adjust-doc",
            params: crate::manual::adjust::AdjustParams,
            run: crate::manual::adjust::run_adjust,
        },
    ]
}
