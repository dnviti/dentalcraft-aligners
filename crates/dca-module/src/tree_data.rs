//! The data of the menu tree and of the manual tools (see `tree` for the id
//! and key naming scheme).

use dc_case::ScanRole;
use dc_module_api::LeafInput;

use crate::tree::Node;

/// Top-level groups of the Aligners menu: one leaf per function, grouped by
/// what it is for, never by "automatic" against "manual" (the manual
/// corrections of a function are tools of its inspector, see [`TOOLS`]).
pub(crate) static GROUPS: &[Node] = &[
    Node::group(
        "aligners.case",
        "aligners-menu-case",
        "aligners-desc-case",
        "folder-open",
        &[
            Node::leaf(
                "aligners.case.import-scans",
                "aligners-menu-case-import-scans",
                "aligners-desc-case-import-scans",
                "upload-simple",
            )
            // a scan from the scanner: an arch or a bite, as a file is
            .scanner(&[ScanRole::Upper, ScanRole::Lower, ScanRole::Bite]),
            Node::leaf(
                "aligners.auto",
                "aligners-menu-auto",
                "aligners-desc-auto",
                "magic-wand",
            )
            .not_a_step(),
        ],
    ),
    Node::group(
        "aligners.teeth",
        "aligners-menu-teeth",
        "aligners-desc-teeth",
        "tooth",
        &[
            Node::leaf(
                "aligners.teeth.find",
                "aligners-menu-teeth-find",
                "aligners-desc-teeth-find",
                "list-numbers",
            ),
            Node::leaf(
                "aligners.preparation.segmentation.automatic",
                "aligners-menu-preparation-segmentation-automatic",
                "aligners-desc-preparation-segmentation-automatic",
                "intersect",
            )
            .key("A"),
            Node::leaf(
                "aligners.teeth.build",
                "aligners-menu-teeth-build",
                "aligners-desc-teeth-build",
                "cube",
            ),
        ],
    ),
    Node::group(
        "aligners.diagnosis",
        "aligners-menu-diagnosis",
        "aligners-desc-diagnosis",
        "stethoscope",
        &[Node::leaf(
            "aligners.diagnosis.space-analysis",
            "aligners-menu-diagnosis-space-analysis",
            "aligners-desc-diagnosis-space-analysis",
            "clipboard-text",
        )],
    ),
    Node::group(
        "aligners.planning",
        "aligners-menu-planning",
        "aligners-desc-planning",
        "target",
        &[
            Node::leaf(
                "aligners.occlusion.align",
                "aligners-menu-occlusion-align",
                "aligners-desc-occlusion-align",
                "arrows-merge",
            ),
            Node::leaf(
                "aligners.planning.setup.automatic",
                "aligners-menu-planning-setup-automatic",
                "aligners-desc-planning-setup-automatic",
                "crosshair",
            ),
            Node::leaf(
                "aligners.planning.aligners",
                "aligners-menu-planning-aligners",
                "aligners-desc-planning-aligners",
                "stack",
            )
            .key("S"),
            // the plugin's page "Revisione clinica", after "Allineatori"
            Node::leaf(
                "aligners.review.classify",
                "aligners-menu-review-classify",
                "aligners-desc-review-classify",
                "list-checks",
            ),
        ],
    ),
    Node::group(
        "aligners.production",
        "aligners-menu-production",
        "aligners-desc-production",
        "printer",
        &[
            Node::leaf(
                "aligners.output.export-all",
                "aligners-menu-output-export-all",
                "aligners-desc-output-export-all",
                "file-arrow-down",
            )
            .input(LeafInput::Folder { param: "dir" })
            .not_a_step(),
            Node::leaf(
                "aligners.output.study-models",
                "aligners-menu-output-study-models",
                "aligners-desc-output-study-models",
                "cube",
            )
            .not_a_step(),
        ],
    ),
    Node::group(
        "aligners.followup",
        "aligners-menu-followup",
        "aligners-desc-followup",
        "clock-counter-clockwise",
        &[
            Node::leaf(
                "aligners.followup.visits",
                "aligners-menu-followup-visits",
                "aligners-desc-followup-visits",
                "calendar-check",
            )
            .input(LeafInput::ScanFile { param: "scan" })
            .not_a_step(),
            Node::leaf(
                "aligners.followup.tracking",
                "aligners-menu-followup-tracking",
                "aligners-desc-followup-tracking",
                "path",
            )
            .not_a_step(),
        ],
    ),
];

/// The commands of the manual corrections that have a label, a description
/// and a key of their own. They are not menu entries: the shell shows each
/// one as a tool in the inspector of the function it corrects, and never
/// runs it on a click (the tools work with the pointer on the scan).
pub(crate) static TOOLS: &[Node] = &[
    Node::leaf(
        "aligners.preparation.numbering.place-seeds",
        "aligners-menu-preparation-numbering-place-seeds",
        "aligners-desc-preparation-numbering-place-seeds",
        "map-pin",
    ),
    Node::leaf(
        "aligners.preparation.numbering.delete-seed",
        "aligners-menu-preparation-numbering-delete-seed",
        "aligners-desc-preparation-numbering-delete-seed",
        "trash",
    ),
    Node::leaf(
        "aligners.preparation.numbering.renumber",
        "aligners-menu-preparation-numbering-renumber",
        "aligners-desc-preparation-numbering-renumber",
        "pencil-simple",
    ),
    Node::leaf(
        "aligners.preparation.numbering.mark-missing",
        "aligners-menu-preparation-numbering-mark-missing",
        "aligners-desc-preparation-numbering-mark-missing",
        "prohibit",
    ),
    Node::leaf(
        "aligners.preparation.segmentation.manual-brush",
        "aligners-menu-preparation-segmentation-manual-brush",
        "aligners-desc-preparation-segmentation-manual-brush",
        "paint-brush",
    )
    .key("B"),
    Node::leaf(
        "aligners.preparation.segmentation.clean-borders",
        "aligners-menu-preparation-segmentation-clean-borders",
        "aligners-desc-preparation-segmentation-clean-borders",
        "broom",
    ),
    Node::leaf(
        "aligners.planning.movements",
        "aligners-menu-planning-movements",
        "aligners-desc-planning-movements",
        "arrows-out-cardinal",
    ),
    Node::leaf(
        "aligners.planning.setup.line",
        "aligners-menu-planning-setup-line",
        "aligners-desc-planning-setup-line",
        "bezier-curve",
    ),
    Node::leaf(
        "aligners.occlusion.bite-registration.three-points",
        "aligners-menu-occlusion-bite-registration-three-points",
        "aligners-desc-occlusion-bite-registration-three-points",
        "hand-pointing",
    ),
];
