//! The printed programme of the patient (`schedule.patient_html`): one table
//! per arch with the change dates and what to do before each aligner.

use std::fmt::Write as _;

use super::html::{self, esc};
use super::text;
use crate::schedule::{Calendar, Date, Lang, Row};

/// What the page shows.
#[derive(Debug, Clone)]
pub struct PatientOptions {
    /// Language of the page.
    pub lang: Lang,
    /// Name of the arch ("upper arch").
    pub arch_label: String,
    /// First aligner to show (`None`: from the first).
    pub first: Option<u32>,
    /// Last aligner to show (`None`: up to the last).
    pub last: Option<u32>,
    /// Next control visit, to end the page with instead of the end date.
    pub visit: Option<Date>,
}

fn in_range(r: &Row, first: Option<u32>, last: Option<u32>) -> bool {
    first.is_none_or(|f| r.aligner >= f) && last.is_none_or(|l| r.aligner <= l)
}

/// The patient programme of one arch as a page.
pub fn patient_html(cal: &Calendar, options: &PatientOptions) -> String {
    let lang = options.lang;
    let t = |k: &'static str| text::tr(lang, k);
    let mut rows = String::new();
    for r in cal
        .rows
        .iter()
        .filter(|r| in_range(r, options.first, options.last))
    {
        let _ = write!(
            rows,
            "<tr><td>&#9744;</td><td>{}{}</td><td>{}</td><td>{}</td><td class=\"l\">{}</td></tr>",
            r.number,
            if r.passive {
                format!(" ({})", esc(t("pat-passive")))
            } else {
                String::new()
            },
            html::date(lang, r.start),
            html::date(lang, r.end),
            esc(&r.todo.join("; ")),
        );
    }
    let total = cal.offset + cal.n_total;
    let head = match options.visit {
        Some(v) => text::fmt(lang, "pat-visit", &[html::date(lang, v)]),
        None => text::fmt(
            lang,
            "pat-end",
            &[html::date(lang, cal.end), total.to_string()],
        ),
    };
    let body = format!(
        "<h1>{}</h1><p>{}</p><table>{}{}</table>",
        text::fmt(lang, "pat-heading", &[esc(&options.arch_label)]),
        text::fmt(
            lang,
            "pat-intro",
            &[html::num(lang, cal.wear_hours, 0), head]
        ),
        html::head(
            lang,
            &[
                "pat-col-done",
                "pat-col-aligner",
                "pat-col-from",
                "pat-col-to",
                "<pat-col-before"
            ]
        ),
        rows,
    );
    html::page(lang, t("pat-title"), &body)
}
