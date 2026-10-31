//! Treatment schedule: aligner change dates and the calendar export (ICS).
//!
//! Reference: `ortho_aligner/schedule.py` (`stage_intervals`, `build`,
//! `next_visit`, `day_of_stage`, `to_ics`).
//!
//! The calendar assumes full compliance (aligner worn at least 22 hours a
//! day, changed exactly on schedule). Interval policy:
//!
//! * `fixed`: every aligner is worn for the same number of days (default 14;
//!   Bollen 2003, Castroflorio 2023);
//! * `adaptive`: 7 days when the aligner only moves anterior teeth by
//!   translation or anterior tipping, 14 days when it contains posterior
//!   angular or vertical movements (Al-Nadawi 2021).
//!
//! Events: attachments bonded before aligner 1, IPR before the aligner in
//! which the contact would otherwise interfere, passive aligners that keep
//! the two arches in step, and the final check (or the next control visit).
//!
//! Dates are calendar dates without a time zone ([`Date`]); the module has no
//! date dependency. The patient-facing text of the calendar is generated in
//! Italian or English ([`Lang`]).
//!
//! Not ported: `patient_html` (the printed patient programme), a later
//! export (WP32).

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use dc_case::Fdi;
use dca_model::Stage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, Serializer};

use crate::anatomy::{self, Arch};
use crate::{evidence, staging};

/// A calendar date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Date {
    /// Year.
    pub year: i32,
    /// Month (1-12).
    pub month: u32,
    /// Day of the month (1-31).
    pub day: u32,
}

impl Date {
    /// Date of `YYYY-MM-DD` (`None` for text that is not a calendar date).
    pub fn parse(text: &str) -> Option<Self> {
        let mut it = text.trim().split('-');
        let year: i32 = it.next()?.parse().ok()?;
        let month: u32 = it.next()?.parse().ok()?;
        let day: u32 = it.next()?.parse().ok()?;
        if it.next().is_some() || !(1..=12).contains(&month) || day == 0 {
            return None;
        }
        (day <= days_in_month(year, month)).then_some(Self { year, month, day })
    }

    /// Today (UTC).
    pub fn today() -> Self {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        Self::from_days((secs / 86_400) as i64)
    }

    /// Days since 1970-01-01.
    pub fn days(self) -> i64 {
        // Hinnant's days_from_civil
        let y = i64::from(self.year) - i64::from(self.month <= 2);
        let era = y.div_euclid(400);
        let yoe = y - era * 400;
        let m = i64::from(self.month);
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + i64::from(self.day) - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }

    /// Date `days` days after 1970-01-01.
    pub fn from_days(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = (yoe + era * 400 + i64::from(month <= 2)) as i32;
        Self { year, month, day }
    }

    /// The date `n` days later (earlier for a negative `n`).
    pub fn add_days(self, n: i64) -> Self {
        Self::from_days(self.days() + n)
    }

    /// `YYYY-MM-DD`.
    pub fn to_iso(self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }

    /// `YYYYMMDD` (iCalendar DATE).
    pub fn compact(self) -> String {
        format!("{:04}{:02}{:02}", self.year, self.month, self.day)
    }

    /// `DD/MM/YYYY`.
    pub fn dmy(self) -> String {
        format!("{:02}/{:02}/{:04}", self.day, self.month, self.year)
    }
}

impl Serialize for Date {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_iso())
    }
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
    }
}

/// How the days of wear of each aligner are chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// Every aligner for the same number of days.
    #[default]
    Fixed,
    /// 7 or 14 days, depending on the movements of the aligner.
    Adaptive,
}

/// Language of the text of the calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Lang {
    /// English.
    #[default]
    En,
    /// Italian.
    It,
}

/// IPR at a stage, with its amount.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IprAt {
    /// First tooth of the contact.
    pub a: Fdi,
    /// Second tooth of the contact.
    pub b: Fdi,
    /// Aligner (1-based) before which the reduction is done.
    pub stage: u32,
    /// Enamel to remove (mm).
    pub mm: f64,
}

/// What the calendar is built from: the staging of one arch.
#[derive(Debug, Clone, Default)]
pub struct Input {
    /// Number of aligners of the arch.
    pub n_stages: u32,
    /// Total aligners including the passive ones that keep the two arches
    /// in step (`None` or smaller than `n_stages`: no passive aligners).
    pub n_total: Option<u32>,
    /// Movement `q` of each tooth at stage 0..=`n_stages` (read by the
    /// adaptive policy).
    pub q: BTreeMap<Fdi, Vec<staging::Q6>>,
    /// IPR with the aligner it is done at.
    pub ipr: Vec<IprAt>,
    /// Teeth that get an attachment (bonded before aligner 1).
    pub attachments: Vec<String>,
    /// Aligners already delivered before this plan (numbering continues).
    pub offset: u32,
}

/// Parameters of the calendar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub struct Params {
    /// Delivery date of the first aligner, `YYYY-MM-DD` (default today).
    pub start_date: Option<String>,
    /// Days of wear of each aligner (fixed policy; also the fallback).
    pub interval_days: u32,
    /// Interval policy.
    pub policy: Policy,
    /// Hours of wear per day.
    pub wear_hours: f64,
    /// Language of the text.
    pub lang: Lang,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            start_date: None,
            interval_days: evidence::param("interval_days")
                .and_then(|p| p.as_i64())
                .map_or(14, |v| v as u32),
            policy: Policy::Fixed,
            wear_hours: evidence::param_f64("wear_hours").unwrap_or(22.0),
            lang: Lang::En,
        }
    }
}

/// Errors of the schedule.
#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ScheduleError {
    /// The start date is not `YYYY-MM-DD`.
    #[error("invalid start date {0:?}: expected YYYY-MM-DD")]
    BadDate(String),
}

/// One aligner of the calendar.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Row {
    /// Aligner of this plan (1-based).
    pub aligner: u32,
    /// Number printed on the aligner (numbering continues after a re-plan).
    pub number: u32,
    /// Passive aligner (the arch repeats its last stage).
    pub passive: bool,
    /// First day of wear.
    pub start: Date,
    /// Last day of wear.
    pub end: Date,
    /// Days of wear.
    pub days: u32,
    /// Treatment day (from 0) the aligner starts on.
    pub first_day: u32,
    /// What to do before starting it (attachments, IPR).
    pub todo: Vec<String>,
}

/// The calendar of one arch.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Calendar {
    /// One row per aligner, passive ones included.
    pub rows: Vec<Row>,
    /// Active aligners.
    pub n_active: u32,
    /// Active and passive aligners.
    pub n_total: u32,
    /// Delivery of the first aligner.
    pub start: Date,
    /// Aligners delivered before this plan.
    pub offset: u32,
    /// The day after the last aligner.
    pub end: Date,
    /// Days of the whole series.
    pub total_days: u32,
    /// Hours of wear per day.
    pub wear_hours: f64,
    /// Interval policy.
    pub policy: Policy,
}

/// Days of wear of aligners 1..=`n` (`stage_intervals`).
pub fn stage_intervals(
    q: &BTreeMap<Fdi, Vec<staging::Q6>>,
    n: u32,
    policy: Policy,
    days: u32,
) -> Vec<u32> {
    if policy != Policy::Adaptive || n == 0 {
        return vec![days; n as usize];
    }
    (1..=n as usize)
        .map(|k| {
            let slow = q.iter().any(|(&f, qs)| {
                if qs.is_empty() {
                    return false;
                }
                let at = |i: usize| qs[i.min(qs.len() - 1)];
                let (a, b) = (at(k), at(k - 1));
                let d: Vec<f64> = (0..6).map(|i| (a[i] - b[i]).abs()).collect();
                if anatomy::is_anterior(f) {
                    d[2] > 1e-6
                } else {
                    d[2] > 1e-6 || d[3..].iter().any(|&x| x > 1e-6)
                }
            });
            if slow { 14 } else { 7 }
        })
        .collect()
}

fn todo_attachments(lang: Lang, list: &str) -> String {
    match lang {
        Lang::It => format!("applicare gli attachment ({list})"),
        Lang::En => format!("bond the attachments ({list})"),
    }
}

/// Full calendar of one arch (`schedule.build`).
pub fn run(input: &Input, params: &Params) -> Result<Calendar, ScheduleError> {
    let start = match &params.start_date {
        Some(text) => Date::parse(text).ok_or_else(|| ScheduleError::BadDate(text.clone()))?,
        None => Date::today(),
    };
    let n = input.n_stages;
    let n_total = n.max(input.n_total.unwrap_or(n));
    let mut iv = stage_intervals(&input.q, n, params.policy, params.interval_days);
    let fill = iv.last().copied().unwrap_or(params.interval_days);
    iv.resize(n_total as usize, fill);
    let mut rows = Vec::with_capacity(n_total as usize);
    let mut day = 0_u32;
    for k in 1..=n_total {
        let days = iv[(k - 1) as usize];
        let d0 = start.add_days(i64::from(day));
        let d1 = d0.add_days(i64::from(days) - 1);
        let mut todo = Vec::new();
        if k == 1 && !input.attachments.is_empty() && input.offset == 0 {
            todo.push(todo_attachments(params.lang, &input.attachments.join(", ")));
        }
        for r in input.ipr.iter().filter(|r| r.stage == k) {
            todo.push(format!("IPR {}|{} {:.2} mm", r.a, r.b, r.mm));
        }
        rows.push(Row {
            aligner: k,
            number: input.offset + k,
            passive: k > n,
            start: d0,
            end: d1,
            days,
            first_day: day,
            todo,
        });
        day += days;
    }
    Ok(Calendar {
        rows,
        n_active: n,
        n_total,
        start,
        offset: input.offset,
        end: start.add_days(i64::from(day)),
        total_days: day,
        wear_hours: params.wear_hours,
        policy: params.policy,
    })
}

/// Control visit after aligner `last` has been worn: the day the next
/// aligner would start (`next_visit`).
pub fn next_visit(cal: &Calendar, last: u32) -> Option<Date> {
    let last = last.clamp(1, cal.rows.len() as u32);
    cal.rows
        .get((last as usize).checked_sub(1)?)
        .map(|r| r.end.add_days(1))
}

/// Treatment day at the (fractional) stage `t` (`day_of_stage`).
pub fn day_of_stage(cal: &Calendar, t: f64) -> f64 {
    if t <= 0.0 || cal.rows.is_empty() {
        return 0.0;
    }
    let k = (t.ceil() as usize).min(cal.rows.len());
    let frac = t - (k as f64 - 1.0);
    let r = &cal.rows[k - 1];
    f64::from(r.first_day) + frac * f64::from(r.days)
}

// ---- from the stages stored in the case ------------------------------------------------------------------------

/// Last aligner in which a tooth of `arch` moves: the aligners after it are
/// passive for this arch.
pub fn active_stages(arch: Arch, stages: &[Stage]) -> u32 {
    let mut last = 0;
    let mut prev: BTreeMap<Fdi, staging::Q6> = BTreeMap::new();
    let mut ordered: Vec<&Stage> = stages.iter().collect();
    ordered.sort_by_key(|s| s.index);
    for s in ordered {
        for (&f, pose) in s.poses.iter().filter(|(f, _)| Arch::of(**f) == arch) {
            let q = staging::q_from_pose(pose);
            let before = prev.get(&f).copied().unwrap_or([0.0; 6]);
            if q.iter().zip(&before).any(|(a, b)| (a - b).abs() > 1e-9) {
                last = last.max(s.index);
            }
            prev.insert(f, q);
        }
    }
    last
}

/// The [`Input`] of one arch from the stages stored in the case. `n_total`
/// is left to the caller (the larger of the arches' series).
pub fn input_from_stages(arch: Arch, stages: &[Stage], attachments: Vec<String>) -> Input {
    let n = active_stages(arch, stages);
    let mut q: BTreeMap<Fdi, Vec<staging::Q6>> = BTreeMap::new();
    let mut ordered: Vec<&Stage> = stages.iter().collect();
    ordered.sort_by_key(|s| s.index);
    for s in &ordered {
        for (&f, pose) in s.poses.iter().filter(|(f, _)| Arch::of(**f) == arch) {
            q.entry(f).or_default();
            let v = q.get_mut(&f).expect("just inserted");
            while v.len() < s.index as usize {
                let last = v.last().copied().unwrap_or([0.0; 6]);
                v.push(last);
            }
            if v.len() == s.index as usize {
                v.push(staging::q_from_pose(pose));
            }
        }
    }
    for v in q.values_mut() {
        if v.is_empty() || v[0] != [0.0; 6] {
            v.insert(0, [0.0; 6]);
        }
        v.truncate(n as usize + 1);
    }
    let mut ipr = Vec::new();
    for s in &ordered {
        for step in &s.ipr {
            if Arch::of(step.between[0]) == arch {
                ipr.push(IprAt {
                    a: step.between[0],
                    b: step.between[1],
                    stage: s.index,
                    mm: step.amount_mm,
                });
            }
        }
    }
    Input {
        n_stages: n,
        n_total: None,
        q,
        ipr,
        attachments,
        offset: 0,
    }
}

// ---- iCalendar ---------------------------------------------------------------------------------------------

/// What the `.ics` of a calendar says and for which aligners.
#[derive(Debug, Clone, PartialEq)]
pub struct IcsOptions {
    /// Name of the case (in the event ids).
    pub title: String,
    /// Name of the arch (in the summaries).
    pub arch_label: String,
    /// First aligner to include (`None`: from the first).
    pub first: Option<u32>,
    /// Last aligner to include (`None`: up to the last).
    pub last: Option<u32>,
    /// Control visit to end with instead of the final check.
    pub visit: Option<Date>,
    /// Language of the text.
    pub lang: Lang,
    /// `DTSTAMP` of the events (`YYYYMMDDTHHMMSSZ`); empty: now.
    pub stamp: String,
}

impl Default for IcsOptions {
    fn default() -> Self {
        Self {
            title: "case".to_owned(),
            arch_label: String::new(),
            first: None,
            last: None,
            visit: None,
            lang: Lang::En,
            stamp: String::new(),
        }
    }
}

fn now_stamp() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let d = Date::from_days((secs / 86_400) as i64);
    let s = secs % 86_400;
    format!(
        "{}T{:02}{:02}{:02}Z",
        d.compact(),
        s / 3600,
        (s / 60) % 60,
        s % 60
    )
}

/// Escapes a TEXT value (RFC 5545, 3.3.11).
fn escape_text(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

/// Folds a content line to at most 75 octets (RFC 5545, 3.1).
fn fold(line: &str) -> String {
    let mut out = String::with_capacity(line.len() + 8);
    let mut cur = 0;
    for ch in line.chars() {
        let l = ch.len_utf8();
        if cur + l > 75 {
            out.push_str("\r\n ");
            cur = 1;
        }
        out.push(ch);
        cur += l;
    }
    out
}

/// Undoes the folding of a content stream (and normalises the line ends to
/// `\n`).
pub fn unfold(s: &str) -> String {
    s.replace("\r\n", "\n").replace("\n ", "")
}

fn uid_part(s: &str) -> String {
    s.replace(' ', "_")
}

fn events(cal: &Calendar, o: &IcsOptions, stamp: &str) -> Vec<String> {
    let total = cal.offset + cal.n_total;
    let mut lines: Vec<String> = Vec::new();
    for r in &cal.rows {
        if o.first.is_some_and(|f| r.aligner < f) || o.last.is_some_and(|l| r.aligner > l) {
            continue;
        }
        let num = r.number;
        let (what, until, before, summary, alarm) = match o.lang {
            Lang::It => (
                if r.passive {
                    "allineatore passivo"
                } else {
                    "allineatore"
                },
                "Indossare l'",
                " Prima: ",
                format!("{}: inizia allineatore {num}/{total}", o.arch_label),
                format!("Domani cambio allineatore {num}"),
            ),
            Lang::En => (
                if r.passive {
                    "passive aligner"
                } else {
                    "aligner"
                },
                "Wear the ",
                " Before: ",
                format!("{}: start aligner {num}/{total}", o.arch_label),
                format!("Tomorrow: change to aligner {num}"),
            ),
        };
        let mut desc = match o.lang {
            Lang::It => format!(
                "{until}{what} {num} ({}) almeno {} ore al giorno fino al {}.",
                o.arch_label,
                cal.wear_hours,
                r.end.dmy()
            ),
            Lang::En => format!(
                "{until}{what} {num} ({}) at least {} hours a day until {}.",
                o.arch_label,
                cal.wear_hours,
                r.end.dmy()
            ),
        };
        if !r.todo.is_empty() {
            desc.push_str(before);
            desc.push_str(&r.todo.join("; "));
            desc.push('.');
        }
        lines.extend([
            "BEGIN:VEVENT".to_owned(),
            uid_line(&format!(
                "dc-{}-{}-{num}@dentalcraft",
                o.title, o.arch_label
            )),
            format!("DTSTAMP:{stamp}"),
            format!("DTSTART;VALUE=DATE:{}", r.start.compact()),
            format!("DTEND;VALUE=DATE:{}", r.start.add_days(1).compact()),
            format!("SUMMARY:{}", escape_text(&summary)),
            format!("DESCRIPTION:{}", escape_text(&desc)),
            "BEGIN:VALARM".to_owned(),
            "TRIGGER:-PT12H".to_owned(),
            "ACTION:DISPLAY".to_owned(),
            format!("DESCRIPTION:{}", escape_text(&alarm)),
            "END:VALARM".to_owned(),
            "END:VEVENT".to_owned(),
        ]);
    }
    let (when, what) = match (o.visit, o.lang) {
        (Some(v), Lang::It) => (
            v,
            "visita di controllo e scansione (analisi della situazione e nuovi allineatori)",
        ),
        (Some(v), Lang::En) => (
            v,
            "control visit and scan (review of the situation and new aligners)",
        ),
        (None, Lang::It) => (cal.end, "fine serie - controllo e scansione finale"),
        (None, Lang::En) => (cal.end, "end of the series - final check and scan"),
    };
    lines.extend([
        "BEGIN:VEVENT".to_owned(),
        uid_line(&format!(
            "dc-{}-{}-{}-check@dentalcraft",
            o.title,
            o.arch_label,
            when.to_iso()
        )),
        format!("DTSTAMP:{stamp}"),
        format!("DTSTART;VALUE=DATE:{}", when.compact()),
        format!("DTEND;VALUE=DATE:{}", when.add_days(1).compact()),
        format!(
            "SUMMARY:{}",
            escape_text(&format!("{}: {what}", o.arch_label))
        ),
        "END:VEVENT".to_owned(),
    ]);
    lines
}

fn uid_line(id: &str) -> String {
    format!("UID:{}", uid_part(id))
}

/// iCalendar file with an event (and a reminder the evening before) for
/// every aligner change, and the final check or the next control visit
/// (`to_ics`).
pub fn to_ics(cal: &Calendar, options: &IcsOptions) -> String {
    to_ics_many(&[(cal, options)])
}

/// One iCalendar file for several arches: the events of each `(calendar,
/// options)`, in the given order. The language and the stamp of the first
/// entry are the file's.
pub fn to_ics_many(entries: &[(&Calendar, &IcsOptions)]) -> String {
    let lang = entries.first().map_or(Lang::En, |(_, o)| o.lang);
    let stamp = entries
        .first()
        .map(|(_, o)| o.stamp.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(now_stamp);
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_owned(),
        "VERSION:2.0".to_owned(),
        format!(
            "PRODID:-//DentalCraft//Aligners//{}",
            match lang {
                Lang::It => "IT",
                Lang::En => "EN",
            }
        ),
        "CALSCALE:GREGORIAN".to_owned(),
    ];
    for (cal, o) in entries {
        lines.extend(events(cal, o, &stamp));
    }
    lines.push("END:VCALENDAR".to_owned());
    let mut out = String::new();
    for l in lines {
        out.push_str(&fold(&l));
        out.push_str("\r\n");
    }
    out
}

/// Checks the basic structure of an iCalendar file as RFC 5545 asks for it:
/// CRLF line ends, content lines of at most 75 octets, a `VCALENDAR` with
/// `VERSION` and `PRODID`, balanced components, and in every `VEVENT` a
/// unique `UID`, a `DTSTAMP`, a `DTSTART` and a `SUMMARY`. Returns the number
/// of events.
pub fn validate_ics(ics: &str) -> Result<usize, String> {
    if !ics.starts_with("BEGIN:VCALENDAR\r\n") || !ics.ends_with("END:VCALENDAR\r\n") {
        return Err("not a VCALENDAR with CRLF line ends".to_owned());
    }
    for line in ics.split("\r\n") {
        if line.len() > 75 {
            return Err(format!("line longer than 75 octets: {line}"));
        }
        if line.contains(['\r', '\n']) {
            return Err("a bare CR or LF".to_owned());
        }
    }
    let text = unfold(ics);
    let mut stack: Vec<String> = Vec::new();
    let mut uids = std::collections::BTreeSet::new();
    // properties seen in the open VEVENT
    let mut event: Option<Vec<String>> = None;
    let (mut events, mut version, mut prodid) = (0, false, false);
    for line in text.lines().filter(|l| !l.is_empty()) {
        let name = line.split([':', ';']).next().unwrap_or_default();
        match name {
            "BEGIN" => {
                let kind = line["BEGIN:".len()..].to_owned();
                if kind == "VEVENT" {
                    events += 1;
                    event = Some(Vec::new());
                }
                stack.push(kind);
            }
            "END" => {
                let kind = &line["END:".len()..];
                if stack.pop().as_deref() != Some(kind) {
                    return Err(format!("unbalanced END:{kind}"));
                }
                if kind == "VEVENT" {
                    let seen = event.take().unwrap_or_default();
                    for need in ["UID", "DTSTAMP", "DTSTART", "SUMMARY"] {
                        if !seen.iter().any(|p| p == need) {
                            return Err(format!("a VEVENT without {need}"));
                        }
                    }
                }
            }
            _ => {
                if stack.last().map(String::as_str) == Some("VCALENDAR") {
                    version |= name == "VERSION" && line == "VERSION:2.0";
                    prodid |= name == "PRODID";
                }
                if stack.last().map(String::as_str) == Some("VEVENT") {
                    if name == "UID" && !uids.insert(line["UID:".len()..].to_owned()) {
                        return Err(format!("duplicate {line}"));
                    }
                    if let Some(seen) = event.as_mut() {
                        seen.push(name.to_owned());
                    }
                }
            }
        }
    }
    if !stack.is_empty() {
        return Err("unclosed component".to_owned());
    }
    if !version || !prodid {
        return Err("VERSION:2.0 and PRODID are required".to_owned());
    }
    Ok(events)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn fdi(n: u8) -> Fdi {
        Fdi::new(n).unwrap()
    }

    #[test]
    fn dates_add_days_across_months_and_leap_years() {
        let d = Date::parse("2026-01-05").unwrap();
        assert_eq!(d.add_days(14).to_iso(), "2026-01-19");
        assert_eq!(d.add_days(210).to_iso(), "2026-08-03");
        assert_eq!(
            Date::parse("2024-02-28").unwrap().add_days(2).to_iso(),
            "2024-03-01"
        );
        assert_eq!(
            Date::parse("2023-02-28").unwrap().add_days(1).to_iso(),
            "2023-03-01"
        );
        assert_eq!(Date::parse("1970-01-01").unwrap().days(), 0);
        assert_eq!(Date::from_days(20_000).days(), 20_000);
        assert_eq!(d.dmy(), "05/01/2026");
        assert_eq!(d.compact(), "20260105");
        assert!(Date::parse("2026-02-30").is_none());
        assert!(Date::parse("2026-13-01").is_none());
        assert!(Date::parse("yesterday").is_none());
        assert!(Date::parse("2026-01-05-1").is_none());
    }

    fn simple(n: u32) -> Input {
        Input {
            n_stages: n,
            ..Input::default()
        }
    }

    #[test]
    fn the_fixed_calendar_adds_the_interval_to_every_aligner() {
        let p = Params {
            start_date: Some("2026-03-01".into()),
            interval_days: 10,
            ..Params::default()
        };
        let cal = run(&simple(3), &p).unwrap();
        assert_eq!(cal.rows.len(), 3);
        assert_eq!(cal.rows[0].start.to_iso(), "2026-03-01");
        assert_eq!(cal.rows[0].end.to_iso(), "2026-03-10");
        assert_eq!(cal.rows[1].start.to_iso(), "2026-03-11");
        assert_eq!(cal.rows[2].first_day, 20);
        assert_eq!(cal.end.to_iso(), "2026-03-31");
        assert_eq!(cal.total_days, 30);
        assert_eq!(next_visit(&cal, 3).unwrap().to_iso(), "2026-03-31");
        assert_eq!(day_of_stage(&cal, 1.5), 15.0);
        assert_eq!(day_of_stage(&cal, 0.0), 0.0);
    }

    #[test]
    fn passive_aligners_fill_up_to_the_total() {
        let p = Params {
            start_date: Some("2026-03-01".into()),
            ..Params::default()
        };
        let input = Input {
            n_total: Some(5),
            ..simple(3)
        };
        let cal = run(&input, &p).unwrap();
        assert_eq!(cal.n_active, 3);
        assert_eq!(cal.n_total, 5);
        assert_eq!(
            cal.rows.iter().map(|r| r.passive).collect::<Vec<_>>(),
            [false, false, false, true, true]
        );
        assert!(cal.rows.iter().all(|r| r.days == 14));
        // no passive aligners when n_total is smaller
        let input = Input {
            n_total: Some(1),
            ..simple(3)
        };
        assert_eq!(run(&input, &p).unwrap().rows.len(), 3);
    }

    #[test]
    fn a_bad_start_date_is_an_error() {
        let p = Params {
            start_date: Some("5/1/2026".into()),
            ..Params::default()
        };
        assert!(matches!(
            run(&simple(1), &p),
            Err(ScheduleError::BadDate(_))
        ));
    }

    #[test]
    fn attachments_and_ipr_go_in_the_to_do_list() {
        let p = Params {
            start_date: Some("2026-03-01".into()),
            lang: Lang::En,
            ..Params::default()
        };
        let input = Input {
            attachments: vec!["13".into(), "23".into()],
            ipr: vec![IprAt {
                a: fdi(11),
                b: fdi(21),
                stage: 2,
                mm: 0.25,
            }],
            ..simple(3)
        };
        let cal = run(&input, &p).unwrap();
        assert_eq!(cal.rows[0].todo, ["bond the attachments (13, 23)"]);
        assert_eq!(cal.rows[1].todo, ["IPR 11|21 0.25 mm"]);
        assert!(cal.rows[2].todo.is_empty());
        // numbering continues after a re-plan, and attachments are not bonded again
        let input = Input { offset: 4, ..input };
        let cal = run(&input, &p).unwrap();
        assert_eq!(cal.rows[0].number, 5);
        assert!(cal.rows[0].todo.is_empty());
    }

    #[test]
    fn the_adaptive_policy_gives_posterior_and_vertical_movements_14_days() {
        let mut q: BTreeMap<Fdi, Vec<staging::Q6>> = BTreeMap::new();
        // an incisor translating, then intruding
        q.insert(
            fdi(11),
            vec![
                [0.0; 6],
                [0.2, 0.0, 0.0, 0.0, 0.0, 0.0],
                [0.4, 0.0, 0.0, 0.0, 0.0, 0.0],
                [0.4, 0.0, -0.2, 0.0, 0.0, 0.0],
            ],
        );
        // a molar tipping in the second aligner
        q.insert(
            fdi(16),
            vec![
                [0.0; 6],
                [0.0; 6],
                [0.0, 0.0, 0.0, 0.0, 0.02, 0.0],
                [0.0, 0.0, 0.0, 0.0, 0.02, 0.0],
            ],
        );
        let iv = stage_intervals(&q, 3, Policy::Adaptive, 14);
        assert_eq!(iv, [7, 14, 14]);
        assert_eq!(stage_intervals(&q, 3, Policy::Fixed, 10), [10, 10, 10]);
        assert!(stage_intervals(&q, 0, Policy::Adaptive, 14).is_empty());
    }

    #[test]
    fn the_stages_of_the_case_give_the_input_of_one_arch() {
        let mut stages = Vec::new();
        for k in 1..=3_u32 {
            let mut s = Stage::new(k);
            // the lower tooth stops moving after aligner 2, the upper one moves to 3
            s.poses.insert(
                fdi(31),
                staging::pose_from_q(&[0.1 * f64::from(k.min(2)), 0.0, 0.0, 0.0, 0.0, 0.0]),
            );
            s.poses.insert(
                fdi(11),
                staging::pose_from_q(&[0.1 * f64::from(k), 0.0, 0.0, 0.0, 0.0, 0.0]),
            );
            if k == 2 {
                s.ipr.push(dca_model::IprStep {
                    between: [fdi(11), fdi(21)],
                    amount_mm: 0.2,
                });
            }
            stages.push(s);
        }
        assert_eq!(active_stages(Arch::Lower, &stages), 2);
        assert_eq!(active_stages(Arch::Upper, &stages), 3);
        let input = input_from_stages(Arch::Lower, &stages, Vec::new());
        assert_eq!(input.n_stages, 2);
        assert_eq!(input.q[&fdi(31)].len(), 3);
        assert!(input.ipr.is_empty());
        let input = input_from_stages(Arch::Upper, &stages, Vec::new());
        assert_eq!(input.q[&fdi(11)].len(), 4);
        assert_eq!(input.ipr.len(), 1);
        assert_eq!(input.ipr[0].stage, 2);
    }

    /// Basic structure of RFC 5545 for the calendars of this module.
    pub(crate) fn check_ics(ics: &str) {
        validate_ics(ics).unwrap();
    }

    #[test]
    fn the_ics_has_an_event_per_aligner_and_a_final_check() {
        let p = Params {
            start_date: Some("2026-03-01".into()),
            ..Params::default()
        };
        let input = Input {
            attachments: vec!["13".into()],
            ..simple(3)
        };
        let cal = run(&input, &p).unwrap();
        let o = IcsOptions {
            title: "Case 1".into(),
            arch_label: "upper arch".into(),
            stamp: "20260101T000000Z".into(),
            ..IcsOptions::default()
        };
        let ics = to_ics(&cal, &o);
        check_ics(&ics);
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 4);
        assert_eq!(ics.matches("BEGIN:VALARM").count(), 3);
        assert!(ics.contains("DTSTART;VALUE=DATE:20260301\r\n"));
        assert!(ics.contains("DTEND;VALUE=DATE:20260302\r\n"));
        // the final check is the day after the last aligner
        assert!(ics.contains("DTSTART;VALUE=DATE:20260412\r\n"));
        // commas in the text are escaped
        assert!(unfold(&ics).contains("bond the attachments (13)"));
        // a range of aligners and a control visit
        let o2 = IcsOptions {
            first: Some(2),
            last: Some(2),
            visit: Date::parse("2026-04-01"),
            ..o.clone()
        };
        let ics = to_ics(&cal, &o2);
        check_ics(&ics);
        assert_eq!(ics.matches("BEGIN:VEVENT").count(), 2);
        assert!(ics.contains("DTSTART;VALUE=DATE:20260401\r\n"));
        // two arches in one file
        let both = to_ics_many(&[
            (&cal, &o),
            (
                &cal,
                &IcsOptions {
                    arch_label: "lower arch".into(),
                    ..o.clone()
                },
            ),
        ]);
        check_ics(&both);
        assert_eq!(both.matches("BEGIN:VEVENT").count(), 8);
    }

    #[test]
    fn long_lines_are_folded_and_text_is_escaped() {
        assert_eq!(unfold(&fold(&"x".repeat(200))), "x".repeat(200));
        assert!(fold(&"é".repeat(100)).split("\r\n").all(|l| l.len() <= 75));
        assert_eq!(unfold(&fold(&"é".repeat(100))), "é".repeat(100));
        assert_eq!(escape_text("a,b;c\\d\ne"), "a\\,b\\;c\\\\d\\ne");
    }
}
