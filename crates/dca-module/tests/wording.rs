//! The words of the app: what a dentist or a technician reads carries no
//! implementation name, no citation and no jargon of the pipeline.

use std::path::PathBuf;

use dc_i18n::Lang;
use serde_json::Value;

/// Names of the implementation, of the papers and of the font: nowhere in
/// the text, documentation of the commands included. Compared in lower case
/// with a whole-word match when `true`.
const NAMES: &[(&str, bool)] = &[
    ("icp", true),
    ("6-dof", false),
    ("blake3", false),
    ("manifold", false),
    ("bvh", true),
    ("voxel", false),
    ("signed distance", false),
    ("distanza con segno", false),
    ("harmonic", false),
    ("armonic", false),
    ("random walk", false),
    ("walker", false),
    ("fairing", false),
    ("polynomial", false),
    ("polinomio", false),
    ("rigid transform", false),
    ("trasformazione rigida", false),
    ("world frame", false),
    ("riferimento mondo", false),
    ("mesh", true),
    ("meshes", true),
    ("lai 2009", false),
    ("zou 2015", false),
    ("liao 2015", false),
    ("kobbelt", false),
    ("alharbi", false),
    ("long 2020", false),
    ("wang 2025", false),
    ("little 1975", false),
    ("bolton 1958", false),
    ("b612", false),
    ("blender", false),
];

/// Words of the pipeline: not in a sentence, but a command may have a
/// parameter with that name, listed after `Parameters:`.
const JARGON: &[(&str, bool)] = &[
    ("seed", true),
    ("seeds", true),
    ("seme", true),
    ("semi", true),
    ("label", true),
    ("labels", true),
    ("etichetta", true),
    ("etichette", true),
    ("segmentation", false),
    ("segmentazione", false),
    ("segmented", false),
    ("segmentat", false),
    ("reconstruction", false),
    ("ricostruzione", false),
    ("staging", false),
    ("stadiazione", false),
    ("crown point cap", false),
    ("running command", false),
    ("command failed", false),
];

fn ftl(lang: Lang) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("i18n")
        .join(lang.code())
        .join("dca-module.ftl");
    std::fs::read_to_string(path).unwrap()
}

/// `(key, text)` of every line of a message: comments are out, and so are
/// the placeables, the names in backticks and the identifiers with an
/// underscore (a parameter such as `voxel_mm` is what scripts type).
fn messages(source: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut key = String::new();
    for line in source.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        let text = match line.split_once(" = ") {
            Some((k, v)) if !k.starts_with(' ') => {
                key = k.to_owned();
                v
            }
            _ => line,
        };
        out.push((key.clone(), plain(text)));
    }
    out
}

fn plain(text: &str) -> String {
    let mut s = String::new();
    let (mut tick, mut brace) = (false, 0);
    for c in text.chars() {
        match c {
            '`' => tick = !tick,
            '{' => brace += 1,
            '}' => brace -= 1,
            c if !tick && brace == 0 => s.push(c),
            _ => {}
        }
    }
    s.split_whitespace()
        .filter(|w| !w.contains('_'))
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn has(text: &str, word: &str, whole: bool) -> bool {
    if whole {
        text.split(|c: char| !c.is_alphanumeric() && c != '-')
            .any(|w| w == word)
    } else {
        text.contains(word)
    }
}

/// The sentences of a documentation key, without its list of parameters.
fn prose(text: &str) -> &str {
    ["parameters:", "parameter:", "parametri:", "parametro:"]
        .iter()
        .filter_map(|m| text.find(m))
        .min()
        .map_or(text, |i| &text[..i])
}

/// `true` when `prefix` starts a command id, not the end of a sentence.
fn has_id(text: &str, prefix: &str) -> bool {
    text.match_indices(prefix).any(|(i, _)| {
        text[i + prefix.len()..]
            .chars()
            .next()
            .is_some_and(char::is_alphabetic)
    })
}

#[test]
fn the_filter_finds_what_it_must_and_spares_the_parameters() {
    assert!(has(&plain("a closed mesh"), "mesh", true));
    assert!(has(&plain("Registered with ICP."), "icp", true));
    assert!(!has(&plain("an explicit role"), "icp", true));
    assert!(!plain("Parameters: voxel_mm, blockout_mm").contains("voxel"));
    assert!(!plain("the `label` of { $seed }").contains("label"));
    assert!(!plain("{ aligners-menu-preparation-segmentation-automatic }").contains("segment"));
    assert_eq!(prose("runs. parameters: seeds"), "runs. ");
    assert!(has_id("run aligners.case.detect-arch first", "aligners."));
    assert!(!has_id("the calendar of the aligners.", "aligners."));
}

#[test]
fn no_implementation_name_or_citation_in_the_text() {
    for lang in [Lang::En, Lang::It] {
        for (key, text) in messages(&ftl(lang)) {
            for (word, whole) in NAMES {
                assert!(
                    !has(&text, word, *whole),
                    "{}: {key} says {word:?}: {text}",
                    lang.code()
                );
            }
        }
    }
}

/// The `description` of every field of a parameter schema: the CLI and the
/// MCP server show them next to the documentation of the command.
fn descriptions(schema: &Value, out: &mut Vec<String>) {
    match schema {
        Value::Object(map) => {
            for (key, value) in map {
                match value {
                    Value::String(text) if key == "description" => out.push(text.clone()),
                    other => descriptions(other, out),
                }
            }
        }
        Value::Array(list) => list.iter().for_each(|v| descriptions(v, out)),
        _ => {}
    }
}

#[test]
fn no_implementation_name_in_the_parameters_of_a_command() {
    let mut found = Vec::new();
    for spec in dca_module::commands::all() {
        let mut texts = Vec::new();
        descriptions(&spec.schema(), &mut texts);
        for text in texts {
            let text = plain(&text);
            for (word, whole) in NAMES {
                if has(&text, word, *whole) {
                    found.push(format!("{}: {word:?}: {text}", spec.id));
                }
            }
        }
    }
    found.dedup();
    assert!(found.is_empty(), "{}", found.join("\n"));
}

#[test]
fn no_jargon_in_a_sentence() {
    for lang in [Lang::En, Lang::It] {
        for (key, text) in messages(&ftl(lang)) {
            for (word, whole) in JARGON {
                assert!(
                    !has(prose(&text), word, *whole),
                    "{}: {key} says {word:?}: {text}",
                    lang.code()
                );
            }
        }
    }
}

#[test]
fn no_command_id_in_what_the_app_shows() {
    for lang in [Lang::En, Lang::It] {
        for (key, text) in messages(&ftl(lang)) {
            // the documentation of a command is for the CLI and the MCP server
            if key.ends_with("-doc") {
                continue;
            }
            assert!(
                !has_id(&text, "aligners.") && !has_id(&text, "case."),
                "{}: {key}: {text}",
                lang.code()
            );
        }
    }
}

#[test]
fn every_leaf_description_is_one_plain_sentence() {
    for lang in [Lang::En, Lang::It] {
        let all = messages(&ftl(lang));
        for leaf in dca_module::tree::leaves() {
            let (_, text) = all
                .iter()
                .find(|(k, _)| k == leaf.desc_key)
                .unwrap_or_else(|| panic!("{}", leaf.desc_key));
            assert!(text.ends_with('.'), "{}: {text}", leaf.desc_key);
            assert!(!text.contains(". "), "{}: {text}", leaf.desc_key);
        }
    }
}
