//! The messages that count something say it with the right plural in both
//! languages (a Fluent selector on the plural category the caller passes).
//!
//! One test in this file: the language is process-wide state.

use dc_i18n::Lang;

/// The warning of a study model that is not one clean solid, for `pieces`.
fn not_solid(pieces: u32) -> String {
    dca_module::loader();
    dc_i18n::tr_args(
        "aligners-study-warn-not-solid",
        &[
            ("arch", "x"),
            ("pieces", &pieces.to_string()),
            ("count", if pieces == 1 { "one" } else { "other" }),
            ("open", "0"),
            ("nonmanifold", "0"),
            ("tunnels", "0"),
        ],
    )
}

#[test]
fn the_pieces_of_a_study_model_are_counted_in_the_right_plural() {
    dc_i18n::set_language(Lang::It).unwrap();
    assert!(not_solid(1).contains("1 pezzo,"), "{}", not_solid(1));
    assert!(not_solid(2).contains("2 pezzi,"), "{}", not_solid(2));
    assert!(not_solid(0).contains("0 pezzi,"), "{}", not_solid(0));
    dc_i18n::set_language(Lang::En).unwrap();
    assert!(not_solid(1).contains("1 piece,"), "{}", not_solid(1));
    assert!(not_solid(3).contains("3 pieces,"), "{}", not_solid(3));
}
