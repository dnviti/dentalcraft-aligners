//! `cfg(optimized)` for code built with optimisations (any `opt-level`
//! above 0, debug assertions or not, as the CI builds its tests): the tests
//! too slow without them run only then (`tests/bite_robustness.rs`).

fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rustc-check-cfg=cfg(optimized)");
    if std::env::var("OPT_LEVEL").is_ok_and(|level| level != "0") {
        println!("cargo::rustc-cfg=optimized");
    }
}
