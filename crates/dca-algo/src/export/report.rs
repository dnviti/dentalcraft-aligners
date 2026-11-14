//! The clinical and technical report of the export (`ortho_aligner/report.py`)
//! as an HTML page in English or Italian.
//!
//! Sections: summary, renders (optional), diagnosis (space analysis,
//! Bolton), set-up (IPR, attachments), movement of every tooth, staging,
//! change calendar, production files and their print check, methods and
//! references. Everything the page says comes from the case: the diagnosis
//! and the staging stored in [`AlignersState`], the calendars of the export
//! and the files it wrote. The text is in [`super::text`].
//!
//! Not ported yet (they need data that the Rust case does not have): the
//! tracking section of a control visit and the visit history (`visits.rs`),
//! the clinician review table, the trim and stamp description (`trim.rs`,
//! `stamp.rs`) and the CAT-CAT items.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use dc_case::Fdi;
use dca_model::{AlignersState, Stage, StagingPattern, TrimMode};
use serde::Serialize;

use super::html::{self, esc, num, png_img};
use super::manifest::Entry;
use super::printcheck::PrintCheck;
use super::{Workflow, local_poses, text};
use crate::anatomy::{self, Arch};
use crate::diagnosis::edit;
use crate::schedule::{Calendar, Lang};
use crate::{evidence, staging};

/// Images embedded in the report, as PNG files.
#[derive(Debug, Clone, Default)]
pub struct Renders {
    /// Occlusal view of the case.
    pub occlusal: Option<Vec<u8>>,
    /// Frontal view of the case.
    pub frontal: Option<Vec<u8>>,
}

/// The print check of one exported mesh.
#[derive(Debug, Clone, Serialize)]
pub struct FileCheck {
    /// File name relative to the export folder.
    pub name: String,
    /// The arch.
    pub arch: Arch,
    /// The stage (`0`: initial situation).
    pub stage: u32,
    /// `model` or `aligner`.
    pub kind: &'static str,
    /// The result.
    pub check: PrintCheck,
}

/// What the report is made of.
#[derive(Debug, Clone)]
pub struct ReportInput<'a> {
    /// Pseudonymised patient code.
    pub patient_code: &'a str,
    /// Name of the case.
    pub title: &'a str,
    /// Date of the export, `YYYY-MM-DD`.
    pub generated: &'a str,
    /// Version of the software.
    pub version: &'a str,
    /// Language of the page.
    pub lang: Lang,
    /// The Aligners data of the case.
    pub state: &'a AlignersState,
    /// `+1` when the x axis of the tooth frame points mesially.
    pub md_sign: &'a BTreeMap<Fdi, i8>,
    /// The calendar of each arch with aligners.
    pub calendars: &'a BTreeMap<Arch, Calendar>,
    /// Teeth with an attachment, by arch (empty: none planned).
    pub attachments: &'a BTreeMap<Arch, Vec<String>>,
    /// The files written so far.
    pub files: &'a [Entry],
    /// The print check of the meshes.
    pub checks: &'a [FileCheck],
    /// The images to embed.
    pub renders: &'a Renders,
    /// Which print files the export makes.
    pub workflow: Workflow,
    /// What could not be done as asked for the print files (text).
    pub notes: &'a [String],
}

/// Movement of one tooth over the whole series.
#[derive(Debug, Clone, PartialEq)]
pub struct ToothRow {
    /// The tooth.
    pub fdi: Fdi,
    /// Total movement: mesial, buccal, extrusion (mm), torque, tip, rotation
    /// (degrees) ([`staging::describe_q`]).
    pub total: [f64; 6],
    /// First aligner that moves it (`0`: never).
    pub first: u32,
    /// Last aligner that moves it.
    pub last: u32,
    /// Largest translation of one aligner (mm).
    pub max_step_mm: f64,
    /// Largest rotation of one aligner (degrees).
    pub max_step_deg: f64,
}

/// The movement of every tooth of an arch.
pub fn tooth_rows(
    stages: &[Stage],
    arch: Arch,
    n_active: u32,
    md_sign: &BTreeMap<Fdi, i8>,
) -> Vec<ToothRow> {
    let present: Vec<Fdi> = stages
        .iter()
        .flat_map(|s| s.poses.keys().copied())
        .filter(|&f| Arch::of(f) == arch)
        .collect();
    let teeth = anatomy::arch_sequence(&present, arch);
    let q_at: Vec<BTreeMap<Fdi, staging::Q6>> = (0..=n_active)
        .map(|k| {
            local_poses(stages, arch, k)
                .iter()
                .map(|(&f, p)| (f, staging::q_from_pose(p)))
                .collect()
        })
        .collect();
    teeth
        .into_iter()
        .map(|f| {
            let q = |k: usize| q_at[k].get(&f).copied().unwrap_or([0.0; 6]);
            let (mut first, mut last, mut max_lin, mut max_ang) = (0, 0, 0.0_f64, 0.0_f64);
            for k in 1..=n_active as usize {
                let (a, b) = (q(k - 1), q(k));
                let lin =
                    ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2) + (b[2] - a[2]).powi(2)).sqrt();
                let ang = ((b[3] - a[3]).powi(2) + (b[4] - a[4]).powi(2) + (b[5] - a[5]).powi(2))
                    .sqrt()
                    .to_degrees();
                if lin > 1e-6 || ang > 1e-4 {
                    if first == 0 {
                        first = k as u32;
                    }
                    last = k as u32;
                }
                max_lin = max_lin.max(lin);
                max_ang = max_ang.max(ang);
            }
            ToothRow {
                fdi: f,
                total: staging::describe_q(
                    &q(n_active as usize),
                    md_sign.get(&f).copied().unwrap_or(1),
                ),
                first,
                last,
                max_step_mm: max_lin,
                max_step_deg: max_ang,
            }
        })
        .collect()
}

fn arch_name(lang: Lang, arch: Arch) -> &'static str {
    text::tr(
        lang,
        if arch.is_upper() {
            "rpt-upper"
        } else {
            "rpt-lower"
        },
    )
}

/// Collects the references cited, to list them at the end.
#[derive(Default)]
struct Cites(Vec<&'static str>);

impl Cites {
    fn cite(&mut self, keys: &[&'static str]) -> String {
        for k in keys {
            if !self.0.contains(k) {
                self.0.push(k);
            }
        }
        format!("({})", esc(&evidence::cite(keys)))
    }
}

/// Builds the report page.
pub fn report_html(input: &ReportInput<'_>) -> String {
    let lang = input.lang;
    let t = |k: &'static str| text::tr(lang, k);
    let f = |k: &'static str, a: &[String]| text::fmt(lang, k, a);
    let n = |x: f64, nd: usize| num(lang, x, nd);
    let mut cites = Cites::default();
    let mut o = String::new();
    let st = input.state;

    let _ = write!(
        o,
        "<h1>{}</h1><div class=\"note\">{}</div><p class=\"disc\">{}</p>",
        esc(t("rpt-title")),
        f(
            "rpt-case-line",
            &[
                esc(input.title),
                esc(input.patient_code),
                esc(input.generated),
                esc(input.version)
            ]
        ),
        esc(t("rpt-disclaimer")),
    );

    // ---- summary ----
    let totals: Vec<(Arch, u32)> = input
        .calendars
        .iter()
        .map(|(a, c)| (*a, c.n_active))
        .collect();
    let total_aligners = input
        .calendars
        .values()
        .map(|c| c.n_total)
        .max()
        .unwrap_or(0);
    let first_cal = input.calendars.values().next();
    let _ = write!(o, "<h2>{}</h2><div class=\"box\">", esc(t("rpt-summary")));
    for (a, k) in &totals {
        let passive = input.calendars[a].n_total.saturating_sub(*k);
        let _ = write!(
            o,
            "<b>{}:</b> {} &nbsp; ",
            esc(arch_name(lang, *a)),
            f("rpt-active", &[k.to_string(), passive.to_string()])
        );
    }
    if let Some(c) = first_cal {
        let months = f64::from(c.total_days) / 30.44;
        let policy = match c.policy {
            crate::schedule::Policy::Fixed => {
                f("rpt-policy-fixed", &[st.settings.interval_days.to_string()])
            }
            crate::schedule::Policy::Adaptive => t("rpt-policy-adaptive").to_owned(),
        };
        let _ = write!(
            o,
            "<br><b>{}:</b> {} &nbsp; <b>{}:</b> {} ({} {}, {} {}) &nbsp; <b>{}:</b> {}",
            esc(t("rpt-total")),
            total_aligners,
            esc(t("rpt-duration")),
            f("rpt-days", &[c.total_days.to_string()]),
            n(months, 1),
            esc(t("rpt-months")),
            esc(t("rpt-until")),
            html::date(lang, c.end),
            esc(t("rpt-change")),
            esc(&policy),
        );
        let _ = write!(
            o,
            "<br><b>{}:</b> {} {}",
            esc(t("rpt-wear")),
            f("rpt-hours", &[n(c.wear_hours, 0)]),
            cites.cite(&["timm2021"]),
        );
        let _ = write!(
            o,
            "<br><span class=\"note\">{} {}</span>",
            esc(t("rpt-interval-note")),
            cites.cite(&[
                "bollen2003",
                "castroflorio2023",
                "alnadawi2021",
                "haouili2020"
            ]),
        );
    }
    if let Some(score) = st.diagnosis.complexity_score {
        let _ = write!(
            o,
            "<br><b>{}:</b> {} {}",
            esc(t("rpt-complexity")),
            n(score, 0),
            cites.cite(&["long2020catcat", "wang2025consensus"]),
        );
    }
    let _ = write!(
        o,
        "<br><b>{}:</b> {}",
        esc(t("rpt-production")),
        esc(t(match input.workflow {
            Workflow::Both => "rpt-wf-both",
            Workflow::Thermo => "rpt-wf-thermo",
            Workflow::Direct => "rpt-wf-direct",
        }))
    );
    o.push_str("</div>");

    // ---- renders ----
    if input.renders.occlusal.is_some() || input.renders.frontal.is_some() {
        let _ = write!(o, "<h2>{}</h2><p>", esc(t("rpt-renders")));
        if let Some(p) = &input.renders.occlusal {
            o.push_str(&png_img(p, t("rpt-occlusal")));
        }
        if let Some(p) = &input.renders.frontal {
            o.push_str(&png_img(p, t("rpt-frontal")));
        }
        o.push_str("</p>");
    }

    // ---- diagnosis ----
    let _ = write!(o, "<h2>{}</h2>", esc(t("rpt-diagnosis")));
    if st.diagnosis.arches.is_empty() {
        let _ = write!(o, "<p class=\"note\">{}</p>", esc(t("rpt-no-diagnosis")));
    } else {
        let threshold = evidence::param_f64("extraction_threshold").unwrap_or(8.0);
        let _ = write!(
            o,
            "<table>{}",
            html::head(
                lang,
                &[
                    "<rpt-arch",
                    "rpt-available",
                    "rpt-required",
                    "rpt-discrepancy",
                    "rpt-little",
                    "rpt-largest-space",
                    "rpt-midline",
                    "rpt-catcat"
                ]
            )
        );
        for (role, sp) in &st.diagnosis.arches {
            let Some(arch) = Arch::from_role(*role) else {
                continue;
            };
            let d = st.diagnosis.details.get(role).copied().unwrap_or_default();
            // the values in force: the clinician's own over the computed ones
            let hand = st.diagnosis.edits_of(*role);
            let mark = |by_hand: bool| if by_hand { " *" } else { "" };
            // discrepancy: + space, - crowding
            let disc = st.diagnosis.discrepancy_mm(*role).unwrap_or(0.0);
            let little = st.diagnosis.little_index_mm(*role).unwrap_or(0.0);
            let score = edit::effective_score(st, *role).unwrap_or(d.catcat_score);
            let class = if -disc > threshold { "warn" } else { "" };
            let _ = write!(
                o,
                "{}<td class=\"l\">{}</td><td>{}</td><td>{}</td><td>{}{}</td><td>{}{}</td><td>{}{}</td><td>{}{}</td><td>{}{}</td></tr>",
                html::tr(class),
                esc(arch_name(lang, arch)),
                n(sp.available_mm, 2),
                n(sp.required_mm, 2),
                n(disc, 2),
                mark(hand.discrepancy_mm.is_some()),
                n(little, 2),
                mark(hand.little_index_mm.is_some()),
                n(st.diagnosis.largest_space_mm(*role), 2),
                mark(hand.largest_space_mm.is_some()),
                n(st.diagnosis.midline_offset_mm(*role), 2),
                mark(hand.midline_offset_mm.is_some()),
                score,
                mark(!hand.is_empty()),
            );
            if -disc > threshold {
                let _ = write!(
                    o,
                    "<tr class=\"bad\"><td class=\"l\" colspan=\"8\">{} {}</td></tr>",
                    esc(&f("rpt-extraction", &[n(threshold, 0)])),
                    cites.cite(&["wang2025consensus"]),
                );
            }
        }
        o.push_str("</table>");
        let _ = write!(
            o,
            "<div class=\"note\">{} {}</div>",
            esc(t("rpt-space-note")),
            cites.cite(&["alharbi2008", "little1975"]),
        );
        if st.diagnosis.edits.values().any(|e| !e.is_empty()) {
            let _ = write!(
                o,
                "<div class=\"note\">{}</div>",
                esc(t("rpt-by-hand-note"))
            );
        }
        if st.diagnosis.bolton_anterior.is_some() || st.diagnosis.bolton_overall.is_some() {
            let mut parts = Vec::new();
            if let Some(b) = st.diagnosis.bolton_overall {
                parts.push(f("rpt-bolton-overall", &[n(b, 1)]));
            }
            if let Some(b) = st.diagnosis.bolton_anterior {
                parts.push(f("rpt-bolton-anterior", &[n(b, 1)]));
            }
            let _ = write!(
                o,
                "<p><b>{}</b> {}: {}. {}</p>",
                esc(t("rpt-bolton")),
                cites.cite(&["bolton1958"]),
                esc(&parts.join("; ")),
                esc(t("rpt-bolton-note")),
            );
        }
    }

    // ---- set-up: IPR and attachments ----
    let _ = write!(o, "<h2>{}</h2><div class=\"box\">", esc(t("rpt-setup")));
    let _ = write!(
        o,
        "{}",
        esc(&f(
            "rpt-setup-line",
            &[
                st.setup.poses.len().to_string(),
                st.setup.locked.len().to_string()
            ]
        ))
    );
    let _ = write!(o, "</div><h2>{}</h2>", esc(t("rpt-ipr")));
    let ipr_stage = ipr_stages(&st.stages);
    if st.setup.ipr.is_empty() {
        let _ = write!(o, "<p>{}</p>", esc(t("rpt-no-ipr")));
    } else {
        let warn = evidence::param_f64("ipr_site_warning").unwrap_or(0.2);
        let _ = write!(
            o,
            "<table>{}",
            html::head(
                lang,
                &[
                    "<rpt-contact",
                    "rpt-total-mm",
                    "rpt-per-surface",
                    "rpt-before-aligner",
                    "<rpt-notes"
                ]
            )
        );
        for step in &st.setup.ipr {
            let [a, b] = step.between;
            let stage = ipr_stage
                .get(&(a.min(b), a.max(b)))
                .map_or_else(|| "-".to_owned(), u32::to_string);
            let note = if step.amount_mm > warn {
                format!(
                    "{} {}",
                    f("rpt-ipr-warn", &[n(warn, 2)]),
                    cites.cite(&["gonzalezgarcia2025"])
                )
            } else {
                String::new()
            };
            let _ = write!(
                o,
                "<tr><td class=\"l\">{a}|{b}</td><td>{}</td><td>{}</td><td>{stage}</td><td class=\"l\">{}</td></tr>",
                n(step.amount_mm, 2),
                n(step.amount_mm / 2.0, 2),
                esc(&note),
            );
        }
        o.push_str("</table>");
        let surface = evidence::param_f64("ipr_surface").unwrap_or(0.25);
        let _ = write!(
            o,
            "<div class=\"note\">{} {}</div>",
            esc(&f("rpt-ipr-limits", &[n(surface, 2)])),
            cites.cite(&[
                "wang2025consensus",
                "firincioglullari2025",
                "zachrisson2007"
            ]),
        );
    }
    let _ = write!(o, "<h2>{}</h2>", esc(t("rpt-attachments")));
    if input.attachments.values().all(Vec::is_empty) {
        let _ = write!(o, "<p>{}</p>", esc(t("rpt-no-attachments")));
    } else {
        o.push_str("<ul>");
        for (a, list) in input.attachments.iter().filter(|(_, l)| !l.is_empty()) {
            let _ = write!(
                o,
                "<li><b>{}:</b> {}</li>",
                esc(arch_name(lang, *a)),
                esc(&list.join(", "))
            );
        }
        o.push_str("</ul>");
    }

    // ---- movements and staging ----
    let _ = write!(o, "<h2>{}</h2>", esc(t("rpt-movements")));
    let mut rows_by_arch: Vec<(Arch, Vec<ToothRow>)> = Vec::new();
    for (&a, c) in input.calendars {
        rows_by_arch.push((a, tooth_rows(&st.stages, a, c.n_active, input.md_sign)));
    }
    for (a, rows) in &rows_by_arch {
        let _ = write!(
            o,
            "<h3>{}</h3><table>{}",
            esc(arch_name(lang, *a)),
            html::head(
                lang,
                &[
                    "rpt-tooth",
                    "rpt-mesial",
                    "rpt-buccal",
                    "rpt-extrusion",
                    "rpt-torque",
                    "rpt-tip",
                    "rpt-rotation",
                    "rpt-aligners",
                    "rpt-max-step"
                ]
            )
        );
        for r in rows {
            let aligners = if r.first == 0 {
                "-".to_owned()
            } else {
                format!("{}-{}", r.first, r.last)
            };
            let _ = write!(
                o,
                "<tr><td>{}</td>{}<td>{aligners}</td><td>{} / {}</td></tr>",
                r.fdi,
                r.total
                    .iter()
                    .enumerate()
                    .fold(String::new(), |mut s, (i, v)| {
                        let _ = write!(s, "<td>{}</td>", n(*v, if i < 3 { 2 } else { 1 }));
                        s
                    }),
                n(r.max_step_mm, 2),
                n(r.max_step_deg, 1),
            );
        }
        o.push_str("</table>");
    }
    let sp = &st.settings.staging;
    let _ = write!(o, "<h2>{}</h2><div class=\"box\">", esc(t("rpt-staging")));
    let _ = write!(
        o,
        "{} {} ",
        esc(&f(
            "rpt-staging-limits",
            &[
                n(sp.step_linear_mm, 2),
                n(sp.step_vertical_mm, 2),
                n(sp.step_rotation_deg, 1),
                n(sp.step_rotation_round_deg, 1),
                n(sp.step_angular_deg, 1),
                n(sp.max_crown_step_mm, 2),
            ]
        )),
        cites.cite(&["castroflorio2023", "simon2014"]),
    );
    let _ = write!(
        o,
        "<br>{}: {}. ",
        esc(t("rpt-pattern")),
        esc(t(match sp.pattern {
            StagingPattern::Auto => "rpt-pat-auto",
            StagingPattern::Proportional => "rpt-pat-proportional",
            StagingPattern::MaxVelocity => "rpt-pat-maxvelocity",
            StagingPattern::SequentialDistal => "rpt-pat-sequential",
            StagingPattern::Structured => "rpt-pat-structured",
        })),
    );
    if sp.collision_check {
        let _ = write!(
            o,
            "{}",
            esc(&f("rpt-collision", &[n(sp.collision_tol_mm, 2)]))
        );
    }
    let _ = write!(
        o,
        "<br>{} {}",
        esc(&f(
            "rpt-shell-line",
            &[
                n(st.settings.shell_thickness_mm, 2),
                n(st.settings.clearance_mm, 2),
                text::tr(lang, trim_key(st.trim.mode)).to_owned(),
            ]
        )),
        cites.cite(&["edelmann2020", "koenig2022", "bucci2019"]),
    );
    o.push_str("</div>");

    // ---- calendar ----
    for (a, c) in input.calendars {
        let _ = write!(
            o,
            "<h2>{} - {}</h2><table>{}",
            esc(t("rpt-calendar")),
            esc(arch_name(lang, *a)),
            html::head(
                lang,
                &[
                    "rpt-aligner",
                    "<rpt-from",
                    "<rpt-to",
                    "rpt-days-col",
                    "<rpt-before"
                ]
            )
        );
        for r in &c.rows {
            let _ = write!(
                o,
                "{}<td>{}{}</td><td class=\"l\">{}</td><td class=\"l\">{}</td><td>{}</td><td class=\"l\">{}</td></tr>",
                html::tr(if r.passive { "note" } else { "" }),
                r.number,
                if r.passive {
                    format!(" ({})", esc(t("pat-passive")))
                } else {
                    String::new()
                },
                html::date(lang, r.start),
                html::date(lang, r.end),
                r.days,
                esc(&r.todo.join("; ")),
            );
        }
        o.push_str("</table>");
    }

    // ---- production ----
    let _ = write!(o, "<h2>{}</h2>", esc(t("rpt-files")));
    let _ = write!(
        o,
        "<table>{}",
        html::head(lang, &["<rpt-file", "<rpt-kind", "rpt-bytes", "<rpt-hash"])
    );
    for e in input.files {
        let _ = write!(
            o,
            "<tr><td class=\"l\">{}</td><td class=\"l\">{}</td><td>{}</td><td class=\"l\">{}</td></tr>",
            esc(&e.name),
            esc(&e.kind),
            e.bytes,
            esc(e.blake3.get(..16).unwrap_or(&e.blake3)),
        );
    }
    o.push_str("</table>");

    // ---- print check ----
    let _ = write!(o, "<h2>{}</h2>", esc(t("rpt-printcheck")));
    if input.checks.is_empty() {
        let _ = write!(o, "<p class=\"note\">{}</p>", esc(t("rpt-no-printcheck")));
    } else {
        let bad = input.checks.iter().filter(|c| !c.check.ok).count();
        let _ = write!(
            o,
            "<p class=\"{}\">{}</p>",
            if bad == 0 { "ok" } else { "bad" },
            esc(&f(
                "rpt-printcheck-sum",
                &[
                    (input.checks.len() - bad).to_string(),
                    input.checks.len().to_string()
                ]
            ))
        );
        let _ = write!(
            o,
            "<table>{}",
            html::head(
                lang,
                &[
                    "<rpt-file",
                    "rpt-volume",
                    "rpt-size",
                    "rpt-thickness",
                    "<rpt-result"
                ]
            )
        );
        for c in input.checks {
            let k = &c.check;
            let result = if k.ok {
                format!("<span class=\"ok\">{}</span>", esc(t("rpt-printable")))
            } else {
                esc(&k.problem_texts(lang).join("; "))
            };
            let _ = write!(
                o,
                "{}<td class=\"l\">{}</td><td>{}</td><td>{}</td><td>{}</td><td class=\"l\">{}</td></tr>",
                html::tr(if k.ok { "" } else { "bad" }),
                esc(&c.name),
                n(k.volume_ml(), 2),
                format_args!(
                    "{} x {} x {}",
                    n(k.size_mm[0], 1),
                    n(k.size_mm[1], 1),
                    n(k.size_mm[2], 1)
                ),
                k.min_thickness_mm
                    .map_or_else(|| "-".to_owned(), |x| n(x, 2)),
                result,
            );
        }
        o.push_str("</table>");
    }

    if !input.notes.is_empty() {
        let _ = write!(
            o,
            "<h2>{}</h2><ul class=\"note\">",
            esc(t("rpt-file-notes"))
        );
        for n in input.notes {
            let _ = write!(o, "<li>{}</li>", esc(n));
        }
        o.push_str("</ul>");
    }

    // ---- methods and references ----
    let _ = write!(
        o,
        "<h2>{}</h2><div class=\"note\">{}</div>",
        esc(t("rpt-methods")),
        esc(t("rpt-methods-text"))
    );
    let _ = write!(
        o,
        "<h2>{}</h2><ol class=\"note\">",
        esc(t("rpt-references"))
    );
    let mut refs: Vec<&evidence::Ref> = cites
        .0
        .iter()
        .filter_map(|k| evidence::reference(k))
        .collect();
    refs.sort_by(|a, b| {
        a.authors
            .to_lowercase()
            .cmp(&b.authors.to_lowercase())
            .then(a.year.cmp(&b.year))
    });
    for r in refs {
        let _ = write!(o, "<li>{}</li>", esc(&r.full()));
    }
    o.push_str("</ol>");
    html::page(lang, t("rpt-title"), &o)
}

fn trim_key(mode: TrimMode) -> &'static str {
    match mode {
        TrimMode::Equator => "rpt-trim-equator",
        TrimMode::Straight => "rpt-trim-straight",
        TrimMode::Scalloped => "rpt-trim-scalloped",
        TrimMode::Smooth => "rpt-trim-smooth",
    }
}

/// Aligner before which each contact's IPR is done (the first stage that
/// lists it), keyed by the ordered pair of teeth.
fn ipr_stages(stages: &[Stage]) -> BTreeMap<(Fdi, Fdi), u32> {
    let mut out = BTreeMap::new();
    let mut ordered: Vec<&Stage> = stages.iter().collect();
    ordered.sort_by_key(|s| s.index);
    for s in ordered {
        for step in &s.ipr {
            let [a, b] = step.between;
            out.entry((a.min(b), a.max(b))).or_insert(s.index);
        }
    }
    out
}
