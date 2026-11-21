//! The print tracking of the aligners (print plan §7.4).
//!
//! [`AlignersState::prints`](crate::AlignersState::prints) holds one
//! [`PrintRecord`] per aligner shell, keyed by [`PrintKey`] (arch, aligner
//! number, passive). An aligner with no record is still to print
//! ("missing"). The records change only through `aligners.print.mark`, one
//! undoable command, and only along the transitions [`print_transition`]
//! allows: `printed` comes only from the user's confirmation or a printer's
//! report, never from the app on its own.

use std::fmt;
use std::str::FromStr;

use dc_case::ScanRole;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// Where an aligner shell is in printing.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum PrintStatus {
    /// On a prepared plate, not sent yet.
    Queued,
    /// Sent to the printer, or its plate file saved for a USB stick.
    Sent,
    /// Printed: the user confirmed it, or the printer reported the job done.
    Printed,
    /// The print failed (printer error or stop).
    Failed,
    /// Printed, but to print again (broken or lost aligner).
    Reprint,
}

impl PrintStatus {
    /// Every status, in workflow order.
    pub const ALL: [PrintStatus; 5] = [
        PrintStatus::Queued,
        PrintStatus::Sent,
        PrintStatus::Printed,
        PrintStatus::Failed,
        PrintStatus::Reprint,
    ];

    /// The name of the status in commands and files (`queued`, ...).
    pub fn as_str(self) -> &'static str {
        match self {
            PrintStatus::Queued => "queued",
            PrintStatus::Sent => "sent",
            PrintStatus::Printed => "printed",
            PrintStatus::Failed => "failed",
            PrintStatus::Reprint => "reprint",
        }
    }
}

/// Who marks an aligner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MarkSource {
    /// The user (a click, or an agent acting on the user's request).
    #[default]
    User,
    /// The printer's own report on a job sent to it.
    Printer,
    /// The app on its own (preparing a plate, sending or saving it).
    App,
}

/// `true` when an aligner whose status is `current` (`None`: never marked,
/// still to print) may be marked `target` by `by`:
///
/// | target | from | by |
/// |---|---|---|
/// | `queued` | missing, `failed`, `reprint` | anyone |
/// | `sent` | missing, `queued`, `sent` (a new job), `failed`, `reprint` | anyone |
/// | `failed` | `queued`, `sent` | anyone |
/// | `reprint` | `printed`, `failed` | the user |
/// | `printed` | anything but `printed` | the user |
/// | `printed` | `sent` | the printer |
///
/// `printed` is never set by the app on its own.
pub fn print_transition(current: Option<PrintStatus>, target: PrintStatus, by: MarkSource) -> bool {
    use PrintStatus::{Failed, Printed, Queued, Reprint, Sent};
    match target {
        Queued => matches!(current, None | Some(Failed | Reprint)),
        Sent => !matches!(current, Some(Printed)),
        Failed => matches!(current, Some(Queued | Sent)),
        Reprint => by == MarkSource::User && matches!(current, Some(Printed | Failed)),
        Printed => match by {
            MarkSource::User => current != Some(Printed),
            MarkSource::Printer => current == Some(Sent),
            MarkSource::App => false,
        },
    }
}

/// One aligner shell to print: its arch, its number in the series and
/// whether it is a passive one (a copy of the last active aligner that
/// keeps the arch in place while the other arch moves on).
///
/// Written as its id: `SUP-03` (upper aligner 3), `INF-12-P` (lower passive
/// aligner 12), as `aligners.print.parts` lists them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrintKey {
    /// The arch ([`ScanRole::Upper`] or [`ScanRole::Lower`]).
    pub arch: ScanRole,
    /// The aligner number, from 1.
    pub stage: u32,
    /// A passive aligner.
    pub passive: bool,
}

impl PrintKey {
    /// The key of aligner `stage` of `arch`; `None` for the bite or
    /// aligner 0 (the initial situation is not an aligner).
    pub fn new(arch: ScanRole, stage: u32, passive: bool) -> Option<Self> {
        (arch != ScanRole::Bite && stage > 0).then_some(Self {
            arch,
            stage,
            passive,
        })
    }

    /// The prefix of the arch in ids and file names (`SUP`, `INF`).
    pub fn prefix(self) -> &'static str {
        if self.arch == ScanRole::Upper {
            "SUP"
        } else {
            "INF"
        }
    }
}

impl fmt::Display for PrintKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}-{:02}", self.prefix(), self.stage)?;
        if self.passive {
            f.write_str("-P")?;
        }
        Ok(())
    }
}

/// A text that is not an aligner id (`SUP-03`, `INF-12-P`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0} is not an aligner id (like SUP-03 or INF-12-P)")]
pub struct BadPrintKey(pub String);

impl FromStr for PrintKey {
    type Err = BadPrintKey;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let bad = || BadPrintKey(s.to_owned());
        let (arch, rest) = if let Some(r) = s.strip_prefix("SUP-") {
            (ScanRole::Upper, r)
        } else if let Some(r) = s.strip_prefix("INF-") {
            (ScanRole::Lower, r)
        } else {
            return Err(bad());
        };
        let (number, passive) = match rest.strip_suffix("-P") {
            Some(n) => (n, true),
            None => (rest, false),
        };
        if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        let stage: u32 = number.parse().map_err(|_| bad())?;
        PrintKey::new(arch, stage, passive).ok_or_else(bad)
    }
}

impl Serialize for PrintKey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for PrintKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// What is known of the print of one aligner shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrintRecord {
    /// Its status.
    pub status: PrintStatus,
    /// When it was marked, seconds since 1970-01-01 UTC.
    pub at: u64,
    /// Who marked it.
    pub by: MarkSource,
    /// The plate or job it went with (the plate file, the printer's task),
    /// when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<String>,
    /// Why, for `failed` and `reprint`, when given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// One mark of [`AlignersState::mark_prints`](crate::AlignersState::mark_prints).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrintMark {
    /// The new status.
    pub status: PrintStatus,
    /// Who marks.
    pub by: MarkSource,
    /// When, seconds since 1970-01-01 UTC.
    pub at: u64,
    /// The plate or job, if any.
    pub job: Option<String>,
    /// Why, if given.
    pub reason: Option<String>,
}

impl crate::AlignersState {
    /// The status of aligner `key` (`None`: still to print).
    pub fn print_status(&self, key: PrintKey) -> Option<PrintStatus> {
        self.prints.get(&key).map(|r| r.status)
    }

    /// Marks every aligner of `keys` with `mark`, all or none: when
    /// [`print_transition`] refuses one, nothing changes and that key is
    /// returned.
    pub fn mark_prints(&mut self, keys: &[PrintKey], mark: &PrintMark) -> Result<(), PrintKey> {
        if let Some(k) = keys
            .iter()
            .find(|k| !print_transition(self.print_status(**k), mark.status, mark.by))
        {
            return Err(*k);
        }
        for k in keys {
            self.prints.insert(
                *k,
                PrintRecord {
                    status: mark.status,
                    at: mark.at,
                    by: mark.by,
                    job: mark.job.clone(),
                    reason: mark.reason.clone(),
                },
            );
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use PrintStatus::{Failed, Printed, Queued, Reprint, Sent};

    #[test]
    fn ids_go_out_and_back() {
        let up3 = PrintKey::new(ScanRole::Upper, 3, false).unwrap();
        let low12p = PrintKey::new(ScanRole::Lower, 12, true).unwrap();
        let big = PrintKey::new(ScanRole::Lower, 104, false).unwrap();
        assert_eq!(up3.to_string(), "SUP-03");
        assert_eq!(low12p.to_string(), "INF-12-P");
        assert_eq!(big.to_string(), "INF-104");
        for k in [up3, low12p, big] {
            assert_eq!(k.to_string().parse::<PrintKey>(), Ok(k));
            let json = serde_json::to_value(k).unwrap();
            assert_eq!(serde_json::from_value::<PrintKey>(json).unwrap(), k);
        }
        assert_eq!("SUP-3".parse::<PrintKey>(), Ok(up3));
        for bad in [
            "",
            "SUP",
            "SUP-",
            "SUP-00",
            "SUP-P",
            "SUP--P",
            "MID-03",
            "sup-03",
            "SUP-03-p",
            "SUP-+3",
            "SUP-03-P-P",
            "SUP-99999999999",
        ] {
            assert!(bad.parse::<PrintKey>().is_err(), "{bad}");
        }
        assert!(PrintKey::new(ScanRole::Bite, 1, false).is_none());
        assert!(PrintKey::new(ScanRole::Upper, 0, false).is_none());
    }

    #[test]
    fn printed_needs_the_user_or_the_printer() {
        let from = [None, Some(Queued), Some(Sent), Some(Failed), Some(Reprint)];
        for c in from {
            assert!(print_transition(c, Printed, MarkSource::User), "{c:?}");
            assert!(!print_transition(c, Printed, MarkSource::App), "{c:?}");
            assert_eq!(
                print_transition(c, Printed, MarkSource::Printer),
                c == Some(Sent),
                "{c:?}"
            );
        }
        for by in [MarkSource::User, MarkSource::Printer, MarkSource::App] {
            assert!(!print_transition(Some(Printed), Printed, by));
        }
    }

    #[test]
    fn the_transitions_follow_the_plan() {
        let app = MarkSource::App;
        let user = MarkSource::User;
        // missing → queued → sent → printed → reprint → queued
        assert!(print_transition(None, Queued, app));
        assert!(print_transition(Some(Queued), Sent, app));
        assert!(print_transition(Some(Sent), Printed, MarkSource::Printer));
        assert!(print_transition(Some(Printed), Reprint, user));
        assert!(print_transition(Some(Reprint), Queued, app));
        // sent → failed → queued; a plate saved straight away is sent
        assert!(print_transition(Some(Sent), Failed, MarkSource::Printer));
        assert!(print_transition(Some(Failed), Queued, app));
        assert!(print_transition(None, Sent, app));
        assert!(print_transition(Some(Sent), Sent, app));
        // what never happens
        assert!(!print_transition(Some(Printed), Queued, app));
        assert!(!print_transition(Some(Printed), Sent, app));
        assert!(!print_transition(Some(Printed), Failed, user));
        assert!(!print_transition(None, Failed, user));
        assert!(!print_transition(None, Reprint, user));
        assert!(!print_transition(Some(Printed), Reprint, app));
        assert!(!print_transition(Some(Queued), Queued, app));
    }

    #[test]
    fn a_record_goes_out_and_back() {
        let r = PrintRecord {
            status: Failed,
            at: 1_790_000_000,
            by: MarkSource::Printer,
            job: Some("plate_01.goo".to_owned()),
            reason: Some("film".to_owned()),
        };
        let v = serde_json::to_value(&r).unwrap();
        assert_eq!(v["status"], "failed");
        assert_eq!(v["by"], "printer");
        assert_eq!(serde_json::from_value::<PrintRecord>(v).unwrap(), r);
        let bare: PrintRecord =
            serde_json::from_str(r#"{"status":"sent","at":5,"by":"app"}"#).unwrap();
        assert_eq!((bare.job, bare.reason), (None, None));
    }

    fn mark(status: PrintStatus, by: MarkSource) -> PrintMark {
        PrintMark {
            status,
            by,
            at: 1_790_000_000,
            job: Some("plate_01.goo".to_owned()),
            reason: None,
        }
    }

    #[test]
    fn a_mark_changes_all_the_aligners_or_none() {
        let a = PrintKey::new(ScanRole::Upper, 1, false).unwrap();
        let b = PrintKey::new(ScanRole::Lower, 1, false).unwrap();
        let mut s = crate::AlignersState::default();
        s.mark_prints(&[a, b], &mark(Sent, MarkSource::App))
            .unwrap();
        s.mark_prints(&[a], &mark(Printed, MarkSource::User))
            .unwrap();
        let before = s.clone();
        // b can be printed, a is already: nothing changes, a is named
        assert_eq!(
            s.mark_prints(&[b, a], &mark(Printed, MarkSource::User)),
            Err(a)
        );
        assert_eq!(s, before);
        assert_eq!(s.print_status(a), Some(Printed));
        assert_eq!(s.print_status(b), Some(Sent));
        assert_eq!(s.prints[&b].job.as_deref(), Some("plate_01.goo"));
        assert_eq!(
            s.print_status(PrintKey::new(ScanRole::Upper, 2, false).unwrap()),
            None
        );
    }

    #[test]
    fn the_tracking_and_the_export_folder_survive_the_case_file() {
        let mut s = crate::AlignersState::default();
        let keys = [
            PrintKey::new(ScanRole::Upper, 3, false).unwrap(),
            PrintKey::new(ScanRole::Lower, 12, true).unwrap(),
        ];
        s.mark_prints(&keys, &mark(Sent, MarkSource::App)).unwrap();
        s.export_dir = Some("/abs/export".to_owned());
        let v = serde_json::to_value(&s).unwrap();
        assert_eq!(v["prints"]["SUP-03"]["status"], "sent");
        assert_eq!(v["prints"]["INF-12-P"]["by"], "app");
        assert_eq!(v["export_dir"], "/abs/export");
        let back: crate::AlignersState = serde_json::from_value(v).unwrap();
        assert_eq!(back, s);
        // a case saved before the tracking has no records and no folder
        let mut old = serde_json::to_value(&s).unwrap();
        old.as_object_mut().unwrap().remove("prints");
        old.as_object_mut().unwrap().remove("export_dir");
        let old: crate::AlignersState = serde_json::from_value(old).unwrap();
        assert!(old.prints.is_empty() && old.export_dir.is_none());
        // nothing tracked: nothing written
        let v = serde_json::to_value(crate::AlignersState::default()).unwrap();
        assert!(v.get("prints").is_none() && v.get("export_dir").is_none());
    }
}

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    fn any_status() -> PrintStatus {
        PrintStatus::ALL[usize::from(kani::any::<u8>() % 5)]
    }

    fn any_current() -> Option<PrintStatus> {
        if kani::any() {
            None
        } else {
            Some(any_status())
        }
    }

    fn any_source() -> MarkSource {
        match kani::any::<u8>() % 3 {
            0 => MarkSource::User,
            1 => MarkSource::Printer,
            _ => MarkSource::App,
        }
    }

    // An aligner becomes printed only by an explicit mark: the user's confirmation, from any
    // status but printed, or the printer's report on a job it was sent. The app on its own
    // never marks one printed.
    #[kani::proof]
    fn printed_only_by_the_user_or_the_printer() {
        let current = any_current();
        let by = any_source();
        if print_transition(current, PrintStatus::Printed, by) {
            assert_ne!(by, MarkSource::App);
            assert_ne!(current, Some(PrintStatus::Printed));
            if by == MarkSource::Printer {
                assert_eq!(current, Some(PrintStatus::Sent));
            }
        }
    }

    // A printed aligner leaves printed only when the user asks to print it again.
    #[kani::proof]
    fn a_printed_aligner_leaves_only_for_a_reprint_by_the_user() {
        let target = any_status();
        let by = any_source();
        if print_transition(Some(PrintStatus::Printed), target, by) {
            assert_eq!(target, PrintStatus::Reprint);
            assert_eq!(by, MarkSource::User);
        }
    }
}
