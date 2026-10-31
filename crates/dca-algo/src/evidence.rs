//! Evidence base of the add-on.
//!
//! Every numeric default used for planning, staging and aligner design is
//! declared here together with the peer-reviewed sources it comes from. The
//! UI tooltips and the case report are generated from this module, so a value
//! can never appear in the software without its bibliographic justification.
//!
//! Reference: `ortho_aligner/evidence.py`. The tables ([`REFERENCES`] and
//! [`PARAMS`]) were generated from the plugin's, entry by entry and in the
//! same order; their texts (findings, notes) are the plugin's Italian ones,
//! kept verbatim because they are bibliographic data and regulatory
//! documentation, not UI strings. Units: mm, degrees, days, hours.
//!
//! A [`Param`] without a verified source is marked `to_confirm` (the plugin's
//! `da_confermare`) and must have an open anomaly in the regulatory file.

use std::cmp::Ordering;

/// A bibliographic reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Ref {
    /// Key used by the parameters.
    pub key: &'static str,
    /// Authors, as printed (`"Surname INITIALS, ..."`).
    pub authors: &'static str,
    /// Year.
    pub year: u16,
    /// Title.
    pub title: &'static str,
    /// Journal or book, with volume and pages.
    pub source: &'static str,
    /// DOI (empty when none).
    pub doi: &'static str,
    /// PubMed id (empty when none).
    pub pmid: &'static str,
    /// What the source found, as far as this software uses it.
    pub finding: &'static str,
}

impl Ref {
    /// `"Surname Year"` of the first author (the initials are dropped).
    pub fn short(&self) -> String {
        let first = self.authors.split(',').next().unwrap_or("");
        let mut tokens: Vec<&str> = first.split_whitespace().collect();
        if tokens.len() > 1 && tokens.last().is_some_and(|t| is_upper(t)) {
            tokens.pop(); // the initials
        }
        format!("{} {}", tokens.join(" "), self.year)
    }

    /// Full citation: authors, year, title, source, DOI and PMID.
    pub fn full(&self) -> String {
        let mut s = format!(
            "{} ({}). {}. {}.",
            self.authors, self.year, self.title, self.source
        );
        if !self.doi.is_empty() {
            s.push_str(&format!(" doi:{}", self.doi));
        }
        if !self.pmid.is_empty() {
            s.push_str(&format!(" PMID:{}", self.pmid));
        }
        s
    }
}

/// Python's `str.isupper`: at least one cased character and no lower-case one.
fn is_upper(s: &str) -> bool {
    s.chars().any(char::is_uppercase) && !s.chars().any(char::is_lowercase)
}

/// The value of a default.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ParamValue {
    /// A real number.
    Float(f64),
    /// A whole number (days, hours, faces, teeth).
    Int(i64),
    /// A table by name (periodontal condition, type of movement).
    Map(&'static [(&'static str, f64)]),
}

/// A default value with its evidence.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Param {
    /// Name of the parameter.
    pub key: &'static str,
    /// The value.
    pub value: ParamValue,
    /// Unit (may be empty).
    pub unit: &'static str,
    /// Keys of the [`REFERENCES`] it comes from.
    pub refs: &'static [&'static str],
    /// What the value means (Italian, as in the plugin).
    pub note: &'static str,
    /// The value has no verified source: an orthodontist has to confirm it.
    pub to_confirm: bool,
}

/// Suffix of the tooltip of a value that is still to be confirmed.
pub const TO_CONFIRM_SUFFIX: &str = " (da confermare da un ortodontista)";

impl Param {
    /// The value as a real number (`None` for a table).
    pub fn as_f64(&self) -> Option<f64> {
        match self.value {
            ParamValue::Float(v) => Some(v),
            ParamValue::Int(v) => Some(v as f64),
            ParamValue::Map(_) => None,
        }
    }

    /// The value as a whole number (`None` for a table or a fraction).
    pub fn as_i64(&self) -> Option<i64> {
        match self.value {
            ParamValue::Int(v) => Some(v),
            ParamValue::Float(v) if v.fract() == 0.0 => Some(v as i64),
            _ => None,
        }
    }

    /// An entry of a table value.
    pub fn entry(&self, name: &str) -> Option<f64> {
        match self.value {
            ParamValue::Map(m) => m.iter().find(|(k, _)| *k == name).map(|(_, v)| *v),
            _ => None,
        }
    }

    /// Tooltip: the note, the short citations and, if needed, the
    /// "to be confirmed" suffix.
    pub fn tooltip(&self) -> String {
        let suffix = if self.to_confirm {
            TO_CONFIRM_SUFFIX
        } else {
            ""
        };
        let sources = if self.refs.is_empty() {
            String::new()
        } else {
            format!(" [{}]", cite(self.refs))
        };
        format!("{}{sources}{suffix}", self.note)
    }
}

/// The reference with this key.
pub fn reference(key: &str) -> Option<&'static Ref> {
    REFERENCES.iter().find(|r| r.key == key)
}

/// The parameter with this key.
pub fn param(key: &str) -> Option<&'static Param> {
    PARAMS.iter().find(|p| p.key == key)
}

/// The numeric value of a parameter (`None` when missing or a table).
pub fn param_f64(key: &str) -> Option<f64> {
    param(key)?.as_f64()
}

/// Short inline citation, e.g. `"Simon 2014; Castroflorio 2023"`. Unknown
/// keys are skipped.
pub fn cite(keys: &[&str]) -> String {
    keys.iter()
        .filter_map(|k| reference(k))
        .map(Ref::short)
        .collect::<Vec<_>>()
        .join("; ")
}

/// Every reference, one per paragraph (full citation then its finding),
/// sorted by authors (case-insensitive) and year.
pub fn references_text() -> String {
    let mut refs: Vec<&Ref> = REFERENCES.iter().collect();
    refs.sort_by(
        |a, b| match a.authors.to_lowercase().cmp(&b.authors.to_lowercase()) {
            Ordering::Equal => a.year.cmp(&b.year),
            o => o,
        },
    );
    let mut lines = Vec::new();
    for r in refs {
        lines.push(r.full());
        if !r.finding.is_empty() {
            lines.push(format!("    -> {}", r.finding));
        }
    }
    lines.join("\n")
}

// Generated from `ortho_aligner/evidence.py` of the plugin (port plan B1.4). The texts are
// the plugin's (Italian) and are kept verbatim: they are bibliographic data, not UI strings.

/// Every reference of the evidence base, in the plugin's order.
pub static REFERENCES: &[Ref] = &[
    Ref {
        key: "castroflorio2023",
        authors: "Castroflorio T, Sedran A, Parrini S, Garino F, Reverdito M, Capuozzo R, Mutinelli \
                     S, Grybauskas S, Vaitiekunas M, Deregibus A",
        year: 2023,
        title: "Predictability of orthodontic tooth movement with aligners: effect of treatment \
                   design",
        source: "Prog Orthod 24(1):2",
        doi: "10.1186/s40510-022-00453-0",
        pmid: "36642743",
        finding: "Valori massimi per stadio: 0,25 mm movimenti lineari, 1 grado movimenti angolari, \
                     2 gradi rotazioni (citato da Martinez-Lozano 2024). Rotazione di denti a corona \
                     tonda: circa 0,4 gradi persi per grado prescritto. Cambio ogni 14 giorni: -12% di \
                     mancata correzione rispetto a 7 giorni.",
    },
    Ref {
        key: "martinezlozano2024",
        authors: "Martinez-Lozano D, Castellanos-Andres D, Lopez-Jimenez AJ",
        year: 2024,
        title: "Staging of Orthodontic Tooth Movement in Clear Aligner Treatment: Macro-Staging and \
                   Micro-Staging - A Narrative Review",
        source: "Appl Sci 14(15):6690",
        doi: "10.3390/app14156690",
        pmid: "",
        finding: "Schema simultaneo (X) vs strutturato (V); i casi gravi beneficiano di uno schema \
                     strutturato; prima i movimenti che creano spazio, i movimenti verticali assoluti \
                     vanno posticipati; distalizzazione sequenziale 1/2 con al massimo due denti per \
                     emiarcata; controllo dei precontatti/sovrapposizioni durante il movimento.",
    },
    Ref {
        key: "simon2014",
        authors: "Simon M, Keilig L, Schwarze J, Jung BA, Bourauel C",
        year: 2014,
        title: "Treatment outcome and efficacy of an aligner technique - regarding incisor torque, \
                   premolar derotation and molar distalization",
        source: "BMC Oral Health 14:68",
        doi: "10.1186/1472-6831-14-68",
        pmid: "24923279",
        finding: "Protocollo: fino a 2 gradi/allineatore rotazione, 1 grado/allineatore torque \
                     incisivi, 0,25 mm/allineatore distalizzazione. Derotazione premolari: accuratezza \
                     41,8% con staging <1,5 gradi/allineatore vs 23,2% sopra; rotazioni >15 gradi: \
                     23,6% vs 43,3%.",
    },
    Ref {
        key: "wang2023",
        authors: "Wang Y, Chen J, Qin S, Han X, Liao L, Sima X, Guo W",
        year: 2023,
        title: "An in vivo evaluation of clear aligners for optimal orthodontic force and movement \
                   to determine high-efficacy and periodontal-friendly aligner staging",
        source: "Heliyon 9(4):e15317",
        doi: "10.1016/j.heliyon.2023.e15317",
        pmid: "37113787",
        finding: "Lo staging clinico piu comune e 0,25 mm per allineatore; intervallo suggerito \
                     0,25-0,33 mm; 0,35 mm con la migliore accuratezza nel modello animale.",
    },
    Ref {
        key: "mali2021",
        authors: "Ma Y, Li S",
        year: 2021,
        title: "The optimal orthodontic displacement of clear aligner for mild, moderate and severe \
                   periodontal conditions: an in vitro study in a periodontally compromised individual \
                   using the finite element model",
        source: "BMC Oral Health 21(1):109",
        doi: "10.1186/s12903-021-01474-7",
        pmid: "33691673",
        finding: "Spostamento ottimale per stadio: <0,18 mm (parodonto sano/lieve), <0,15 mm \
                     (moderato), <0,10 mm (grave).",
    },
    Ref {
        key: "yang2025",
        authors: "Yang S, Cheng Y",
        year: 2025,
        title: "Force-Driven Model for Automated Clear Aligner Staging Design Based on Stepwise \
                   Tooth Displacement and Rotation in 3D Space",
        source: "Bioengineering 12(2):111",
        doi: "10.3390/bioengineering12020111",
        pmid: "40001631",
        finding: "Staging automatico: nuovo stadio quando lo spostamento massimo cumulativo \
                     raggiunge 0,25 mm; gestione delle collisioni.",
    },
    Ref {
        key: "liu2025pf",
        authors: "Liu Y, Ma Y, Yang L, Zhang C, Wei G, Chen R, Gu M, Pan J, Yang Z, Komura T, Xin S, \
                     Zhou Y, Tu C, Wang W",
        year: 2025,
        title: "A Potential Field Method for Tooth Motion Planning in Orthodontic Treatment",
        source: "IEEE Trans Vis Comput Graph 31(10):8480-8490",
        doi: "10.1109/TVCG.2025.3567299",
        pmid: "40388270",
        finding: "Pianificazione del percorso dei denti senza collisioni tra denti adiacenti.",
    },
    Ref {
        key: "kravitz2009",
        authors: "Kravitz ND, Kusnoto B, BeGole E, Obrez A, Agran B",
        year: 2009,
        title: "How well does Invisalign work? A prospective clinical study evaluating the efficacy \
                   of tooth movement with Invisalign",
        source: "Am J Orthod Dentofacial Orthop 135(1):27-35",
        doi: "10.1016/j.ajodo.2007.05.018",
        pmid: "19121497",
        finding: "Accuratezza media 41%; estrusione il movimento meno accurato (29,6%).",
    },
    Ref {
        key: "haouili2020",
        authors: "Haouili N, Kravitz ND, Vaid NR, Ferguson DJ, Makki L",
        year: 2020,
        title: "Has Invisalign improved? A prospective follow-up study on the efficacy of tooth \
                   movement with Invisalign",
        source: "Am J Orthod Dentofacial Orthop 158(3):420-425",
        doi: "10.1016/j.ajodo.2019.12.015",
        pmid: "32620479",
        finding: "Accuratezza media 50%; rotazione la meno accurata (46%); intrusione incisivi \
                     inferiori 35%.",
    },
    Ref {
        key: "koletsi2021",
        authors: "Koletsi D, Iliadi A, Eliades T",
        year: 2021,
        title: "Predictability of rotational tooth movement with orthodontic aligners comparing \
                   software-based and achieved data: A systematic review and meta-analysis of \
                   observational studies",
        source: "J Orthod 48(3):277-287",
        doi: "10.1177/14653125211027266",
        pmid: "34176358",
        finding: "Canini superiori con la minore accuratezza di rotazione (47,9%).",
    },
    Ref {
        key: "kravitz2008",
        authors: "Kravitz ND, Kusnoto B, Agran B, Viana G",
        year: 2008,
        title: "Influence of attachments and interproximal reduction on the accuracy of canine \
                   rotation with Invisalign",
        source: "Angle Orthod 78(4):682-687",
        doi: "10.2319/0003-3219(2008)078[0682:IOAAIR]2.0.CO;2",
        pmid: "18302468",
        finding: "Accuratezza media della rotazione dei canini 35,8%.",
    },
    Ref {
        key: "rossini2015",
        authors: "Rossini G, Parrini S, Castroflorio T, Deregibus A, Debernardi CL",
        year: 2015,
        title: "Efficacy of clear aligners in controlling orthodontic tooth movement: a systematic \
                   review",
        source: "Angle Orthod 85(5):881-889",
        doi: "10.2319/061614-436.1",
        pmid: "25412265",
        finding: "Revisione sistematica.",
    },
    Ref {
        key: "papadimitriou2018",
        authors: "Papadimitriou A, Mousoulea S, Gkantidis N, Kloukos D",
        year: 2018,
        title: "Clinical effectiveness of Invisalign orthodontic treatment: a systematic review",
        source: "Prog Orthod 19(1):37",
        doi: "10.1186/s40510-018-0235-z",
        pmid: "30264270",
        finding: "Revisione sistematica.",
    },
    Ref {
        key: "palone2023",
        authors: "Palone M, Pignotti A, Morin E, Pancari C, Spedicato GA, Cremonini F, Lombardo L",
        year: 2023,
        title: "Analysis of overcorrection to be included for planning clear aligner therapy: a \
                   retrospective study",
        source: "Angle Orthod 93(1):11-18",
        doi: "10.2319/052022-371.1",
        pmid: "36223202",
        finding: "Movimenti correttivi in % del prescritto: inclinazione 20,5%, angolazione 14,5%, \
                     rotazione 28,4%, estrusione 11,7%, intrusione 22%; suggerita ipercorrezione ~20%.",
    },
    Ref {
        key: "bollen2003",
        authors: "Bollen AM, Huang G, King G, Hujoel P, Ma T",
        year: 2003,
        title: "Activation time and material stiffness of sequential removable orthodontic \
                   appliances. Part 1: Ability to complete treatment",
        source: "Am J Orthod Dentofacial Orthop 124(5):496-501",
        doi: "10.1016/S0889-5406(03)00576-6",
        pmid: "14614415",
        finding: "Cambio ogni 2 settimane: maggiore probabilita di completare la serie (37% vs 21%).",
    },
    Ref {
        key: "alnadawi2021",
        authors: "Al-Nadawi M, Kravitz ND, Hansa I, Makki L, Ferguson DJ, Vaid NR",
        year: 2021,
        title: "Effect of clear aligner wear protocol on the efficacy of tooth movement",
        source: "Angle Orthod 91(2):157-163",
        doi: "10.2319/071520-630.1",
        pmid: "33296455",
        finding: "7, 10 e 14 giorni equivalenti per i movimenti anteriori; 14 giorni piu accurato \
                     per alcuni movimenti posteriori.",
    },
    Ref {
        key: "timm2021",
        authors: "Timm LH, Farrag G, Baxmann M, Schwendicke F",
        year: 2021,
        title: "Factors Influencing Patient Compliance during Clear Aligner Therapy: A Retrospective \
                   Cohort Study",
        source: "J Clin Med 10(14):3103",
        doi: "10.3390/jcm10143103",
        pmid: "34300269",
        finding: "Compliance definita come uso >= 22 ore/giorno.",
    },
    Ref {
        key: "ravera2016",
        authors: "Ravera S, Castroflorio T, Garino F, Daher S, Cugliari G, Deregibus A",
        year: 2016,
        title: "Maxillary molar distalization with aligners in adult patients: a multicenter \
                   retrospective study",
        source: "Prog Orthod 17:12",
        doi: "10.1186/s40510-016-0126-0",
        pmid: "27041551",
        finding: "Distalizzazione sequenziale: 2,25 mm del primo molare senza tipping significativo.",
    },
    Ref {
        key: "wang2025consensus",
        authors: "Wang Y, Long H, Zhao Z, Bai D, Han X, Wang J, Fang B, Jin Z, He H, Bai Y, Li W, Hu \
                     M, Zhou Y, Ai H, Liu Y, Cao Y, Lin J, Li H, Guo J, Lai W",
        year: 2025,
        title: "Expert consensus on the clinical strategies for orthodontic treatment with clear \
                   aligners",
        source: "Int J Oral Sci 17(1):19",
        doi: "10.1038/s41368-025-00350-2",
        pmid: "40074738",
        finding: "Espansione fino a 2 mm per lato sicura nella maggior parte dei casi; \
                     distalizzazione <2 mm per lato prevedibile; 1 mm di proinclinazione (2,5 gradi) \
                     fornisce 2 mm di spazio; IPR max 0,25 mm per superficie prossimale e <=50% dello \
                     smalto; discrepanza >8 mm indica estrazione; power ridge se >3 gradi di torque \
                     radicolare.",
    },
    Ref {
        key: "long2020catcat",
        authors: "Long H, Wu Z, Yan X, et al.",
        year: 2020,
        title: "An objective system for appraising clear aligner treatment difficulty: clear aligner \
                   treatment complexity assessment tool (CAT-CAT)",
        source: "BMC Oral Health 20(1):312",
        doi: "10.1186/s12903-020-01300-6",
        pmid: "33167950",
        finding: "Punteggi: affollamento 0-3 mm=0, 3-5=2, 5-7=3, >7=4; rotazione premolari/incisivi \
                     laterali 2 punti ogni 10 gradi; distalizzazione molari <2 mm=0, 2-5=3, >5=7; \
                     mesializzazione <2=3, 2-5=7, >5=12; deviazione mediana (x-2)*2; diastema >2 mm: 2 \
                     punti per mm aggiuntivo.",
    },
    Ref {
        key: "braun1998",
        authors: "Braun S, Hnat WP, Fender DE, Legan HL",
        year: 1998,
        title: "The form of the human dental arch",
        source: "Angle Orthod 68(1):29-36",
        doi: "10.1043/0003-3219(1998)068<0029:TFOTHD>2.3.CO;2",
        pmid: "9503132",
        finding: "Funzione beta: y = 3,0314 D [(1/2 + x/W)(1/2 - x/W)]^0,8 (r medio 0,98).",
    },
    Ref {
        key: "alharbi2008",
        authors: "AlHarbi S, Alkofide EA, AlMadi A",
        year: 2008,
        title: "Mathematical analyses of dental arch curvature in normal occlusion",
        source: "Angle Orthod 78(2):281-287",
        doi: "10.2319/121806-516.1",
        pmid: "18251619",
        finding: "Il polinomio di 4 grado descrive bene la curvatura liscia dell'arcata.",
    },
    Ref {
        key: "noroozi2001",
        authors: "Noroozi H, Hosseinzadeh Nik T, Saeeda R",
        year: 2001,
        title: "The dental arch form revisited",
        source: "Angle Orthod 71(5):386-389",
        doi: "10.1043/0003-3219(2001)071<0386:TDAFR>2.0.CO;2",
        pmid: "11605873",
        finding: "Polinomio Y = AX^6 + BX^2 come sostituto della funzione beta.",
    },
    Ref {
        key: "little1975",
        authors: "Little RM",
        year: 1975,
        title: "The irregularity index: a quantitative score of mandibular anterior alignment",
        source: "Am J Orthod 68(5):554-563",
        doi: "10.1016/0002-9416(75)90086-X",
        pmid: "1059332",
        finding: "Somma degli spostamenti lineari dei punti di contatto anatomici dei 6 denti \
                     anteriori.",
    },
    Ref {
        key: "bolton1958",
        authors: "Bolton WA",
        year: 1958,
        title: "Disharmony in tooth size and its relation to the analysis and treatment of \
                   malocclusion",
        source: "Angle Orthod 28(3):113-130",
        doi: "",
        pmid: "",
        finding: "Rapporto anteriore 77,2%, totale 91,3%.",
    },
    Ref {
        key: "germane1991",
        authors: "Germane N, Lindauer SJ, Rubenstein LK, Revere JH, Isaacson RJ",
        year: 1991,
        title: "Increase in arch perimeter due to orthodontic expansion",
        source: "Am J Orthod Dentofacial Orthop 100(5):421-427",
        doi: "10.1016/0889-5406(91)70081-7",
        pmid: "1815557",
        finding: "L'avanzamento incisivo e circa 4 volte piu efficace dell'espansione molare per il \
                     perimetro.",
    },
    Ref {
        key: "hnat2000",
        authors: "Hnat WP, Braun S, Chinhara A, Legan HL",
        year: 2000,
        title: "The relationship of arch length to alterations in dental arch width",
        source: "Am J Orthod Dentofacial Orthop 118(2):184-188",
        doi: "10.1067/mod.2000.105570",
        pmid: "10935959",
        finding: "Modello matematico lunghezza/larghezza di arcata.",
    },
    Ref {
        key: "burke1998",
        authors: "Burke SP, Silveira AM, Goldsmith LJ, et al.",
        year: 1998,
        title: "A meta-analysis of mandibular intercanine width in treatment and postretention",
        source: "Angle Orthod 68(1):53-60",
        doi: "10.1043/0003-3219(1998)068<0053:AMAOMI>2.3.CO;2",
        pmid: "9503135",
        finding: "La larghezza intercanina mandibolare espansa tende a tornare al valore originale.",
    },
    Ref {
        key: "houle2017",
        authors: "Houle JP, Piedade L, Todescan R Jr, Pinheiro FH",
        year: 2017,
        title: "The predictability of transverse changes with Invisalign",
        source: "Angle Orthod 87(1):19-24",
        doi: "10.2319/122115-875.1",
        pmid: "27304231",
        finding: "Accuratezza dell'espansione mascellare 72,8%, piu tipping.",
    },
    Ref {
        key: "zhou2020",
        authors: "Zhou N, Guo J",
        year: 2020,
        title: "Efficiency of upper arch expansion with the Invisalign system",
        source: "Angle Orthod 90(1):23-30",
        doi: "10.2319/022719-151.1",
        pmid: "31368778",
        finding: "Efficienza espansione: canino 79,8%, premolari 73-76%, primo molare 68,3%.",
    },
    Ref {
        key: "yared2006",
        authors: "Yared KF, Zenobio EG, Pacheco W",
        year: 2006,
        title: "Periodontal status of mandibular central incisors after orthodontic proclination in \
                   adults",
        source: "Am J Orthod Dentofacial Orthop 130(1):6.e1-8",
        doi: "10.1016/j.ajodo.2006.01.015",
        pmid: "16849063",
        finding: "Inclinazione finale >95 gradi e gengiva sottile associate a recessioni.",
    },
    Ref {
        key: "artun1987",
        authors: "Artun J, Krogstad O",
        year: 1987,
        title: "Periodontal status of mandibular incisors following excessive proclination",
        source: "Am J Orthod Dentofacial Orthop 91(3):225-232",
        doi: "10.1016/0889-5406(87)90450-1",
        pmid: "3469907",
        finding: "Proinclinazione >10 gradi associata a piu recessioni.",
    },
    Ref {
        key: "handelman1996",
        authors: "Handelman CS",
        year: 1996,
        title: "The anterior alveolus: its importance in limiting orthodontic treatment and its \
                   influence on the occurrence of iatrogenic sequelae",
        source: "Angle Orthod 66(2):95-109",
        doi: "10.1043/0003-3219(1996)066<0095:TAAIII>2.3.CO;2",
        pmid: "8712499",
        finding: "L'osso alveolare sottile limita i movimenti sagittali degli incisivi.",
    },
    Ref {
        key: "andrews1972",
        authors: "Andrews LF",
        year: 1972,
        title: "The six keys to normal occlusion",
        source: "Am J Orthod 62(3):296-309",
        doi: "10.1016/S0002-9416(72)90268-0",
        pmid: "4505873",
        finding: "Obiettivi occlusali (contatti stretti, niente rotazioni).",
    },
    Ref {
        key: "casko1998",
        authors: "Casko JS, et al.",
        year: 1998,
        title: "Objective grading system for dental casts and panoramic radiographs",
        source: "Am J Orthod Dentofacial Orthop 114(5):589-599",
        doi: "10.1016/S0889-5406(98)70179-9",
        pmid: "9810056",
        finding: "ABO-OGS: allineamento e contatti 0,5-1 mm = 1 punto, >1 mm = 2 punti.",
    },
    Ref {
        key: "wheeler",
        authors: "Nelson SJ",
        year: 2019,
        title: "Wheeler's Dental Anatomy, Physiology and Occlusion",
        source: "Elsevier, St. Louis",
        doi: "",
        pmid: "",
        finding: "Lunghezze medie di corona e radice e diametri mesiodistali.",
    },
    Ref {
        key: "poppe2002",
        authors: "Poppe M, Bourauel C, Jager A",
        year: 2002,
        title: "Determination of the elasticity parameters of the human periodontal ligament and the \
                   location of the center of resistance of single-rooted teeth",
        source: "J Orofac Orthop 63(5):358-370",
        doi: "10.1007/s00056-002-0067-8",
        pmid: "12297965",
        finding: "Centro di resistenza al 42% dell'altezza alveolare dalla cresta verso l'apice.",
    },
    Ref {
        key: "gargiulo1961",
        authors: "Gargiulo AW, Wentz FM, Orban B",
        year: 1961,
        title: "Dimensions and relations of the dentogingival junction in humans",
        source: "J Periodontol 32:261-267",
        doi: "10.1902/jop.1961.32.3.261",
        pmid: "",
        finding: "Solco 0,69 mm + epitelio giunzionale 0,97 mm + connettivo 1,07 mm (margine-cresta \
                     ~2,73 mm).",
    },
    Ref {
        key: "firincioglullari2025",
        authors: "Firinciogullari EC, Erdinc AE, Akyalcin S",
        year: 2025,
        title: "3-Dimensional Evaluation of Enamel Thickness to Guide Orthodontic Interproximal \
                   Reduction: A CBCT-Based Study Across Gender and Ethnicity",
        source: "Turk J Orthod 38(2):89-96",
        doi: "10.4274/TurkJOrthod.2025.2025.36",
        pmid: "",
        finding: "Spessore medio dello smalto prossimale per dente e superficie.",
    },
    Ref {
        key: "gonzalezgarcia2025",
        authors: "Gonzalez-Garcia E, Balut-Chahin N, Rojo-Arce CD, Corona MEJ, Cruz-Hervert LP, \
                     Retrouvey JM",
        year: 2025,
        title: "Assessing Enamel Thickness to Estimate Interproximal Reduction: A CBCT-Based Study",
        source: "Clin Exp Dent Res 11(1):e70083",
        doi: "10.1002/cre2.70083",
        pmid: "",
        finding: "Cautela per IPR in un singolo sito oltre 0,20 mm.",
    },
    Ref {
        key: "zachrisson2007",
        authors: "Zachrisson BU, Nyoygaard L, Mobarak K",
        year: 2007,
        title: "Dental health assessed more than 10 years after interproximal enamel reduction of \
                   mandibular anterior teeth",
        source: "Am J Orthod Dentofacial Orthop 131(2):162-169",
        doi: "10.1016/j.ajodo.2006.10.001",
        pmid: "17276856",
        finding: "Nessuna nuova carie a >10 anni dopo IPR.",
    },
    Ref {
        key: "defelice2020",
        authors: "De Felice ME, Nucci L, Fiori A, Flores-Mir C, Perillo L, Grassia V",
        year: 2020,
        title: "Accuracy of interproximal enamel reduction during clear aligner treatment",
        source: "Prog Orthod 21(1):28",
        doi: "10.1186/s40510-020-00329-1",
        pmid: "32719906",
        finding: "L'IPR eseguita e in media inferiore alla pianificata.",
    },
    Ref {
        key: "elshazly2022",
        authors: "Elshazly TM, Keilig L, Salvatori D, et al.",
        year: 2022,
        title: "Effect of trimming line design and edge extension of orthodontic aligners on force \
                   transmission: An in vitro study",
        source: "J Dent 125:104276",
        doi: "10.1016/j.jdent.2022.104276",
        pmid: "36055460",
        finding: "Bordo dritto esteso: forze piu alte; bordo festonato: le piu basse.",
    },
    Ref {
        key: "elshazly2023",
        authors: "Elshazly TM, Salvatori D, Elattar H, et al.",
        year: 2023,
        title: "Effect of trimming line design and edge extension of orthodontic aligners on force \
                   transmission: A 3D finite element study",
        source: "J Mech Behav Biomed Mater 140:105741",
        doi: "10.1016/j.jmbbm.2023.105741",
        pmid: "36857974",
        finding: "Il bordo dritto esteso distribuisce meglio le forze (favorisce il movimento \
                     corporeo).",
    },
    Ref {
        key: "elshazly2024",
        authors: "Elshazly TM, Bourauel C, Aldesoki M, et al.",
        year: 2024,
        title: "Effect of attachment configuration and trim line design on the force system of \
                   orthodontic aligners: A finite element study on the upper central incisor",
        source: "Orthod Craniofac Res 27(Suppl 2):131-140",
        doi: "10.1111/ocr.12779",
        pmid: "38459802",
        finding: "Attachment rettangolari orizzontali cruciali per l'estrusione; bordo dritto esteso \
                     forze maggiori.",
    },
    Ref {
        key: "traversa2025",
        authors: "Traversa F, Chavanne P, Mah J",
        year: 2025,
        title: "Biomechanics of clear aligner therapy: Assessing the influence of tooth position and \
                   flat trimline height in translational movements",
        source: "Orthod Craniofac Res 28(1):1-11",
        doi: "10.1111/ocr.12796",
        pmid: "38685814",
        finding: "Bordo piatto esteso 2 mm (TL2) con miglior controllo rispetto al margine (TL0).",
    },
    Ref {
        key: "cowley2012",
        authors: "Cowley DP, Mah J, O'Toole B",
        year: 2012,
        title: "The effect of gingival-margin design on the retention of thermoformed aligners",
        source: "J Clin Orthod 46(11):697-702",
        doi: "",
        pmid: "23395874",
        finding: "Fogli Invisacryl A e C da 1 mm: bordi dritti (allo zenit gengivale e 2 mm oltre) \
                     piu ritentivi del festonato; il piu ritentivo e il bordo dritto 2 mm oltre lo \
                     zenit con attachment; i bordi dritti riducono la flessibilita dell'allineatore.",
    },
    Ref {
        key: "nakornnoi2024",
        authors: "Nakornnoi T, Srirodjanakul W, Chintavalakorn R, Santiwong P, Sipiyaruk K",
        year: 2024,
        title: "The biomechanical effects of clear aligner trimline designs and extensions on \
                   orthodontic tooth movement: a systematic review",
        source: "BMC Oral Health 24(1):1523",
        doi: "10.1186/s12903-024-05274-7",
        pmid: "39707339",
        finding: "Revisione sistematica (12 studi): bordi dritti ed estesi danno forze e momenti \
                     maggiori e una ritenzione superiore rispetto a bordi festonati o piu corti.",
    },
    Ref {
        key: "bortoli2026",
        authors: "Bortoli NA, Picolli Bernd G, Radke Bittencourt H, Spohr AM, Blessmann Weber JB",
        year: 2026,
        title: "Retention of orthodontic aligners with different gingival margin designs on \
                   three-dimensionally printed models representing varying degrees of marginal tissue \
                   recession",
        source: "Korean J Orthod 56(3):200-206",
        doi: "10.4041/kjod25.269",
        pmid: "42045106",
        finding: "Forza di distacco: festonato esteso 52,3 N e dritto 47,9 N contro festonato 33,1 \
                     N; la ritenzione cresce con la recessione del margine (classe IV 82,2 N, controllo \
                     26,8 N).",
    },
    Ref {
        key: "tran2026",
        authors: "Tran TBV, Pham TNC",
        year: 2026,
        title: "Retentive force of thermoformed and direct 3D-printed clear aligners with different \
                   margin designs and attachments: an in vitro study",
        source: "Front Oral Health 7:1881161",
        doi: "10.3389/froh.2026.1881161",
        pmid: "42488135",
        finding: "Termoformati Zendura VIVA 0,89 mm e stampati Tera Harz TC-85DAC da 0,50 mm di \
                     progetto: bordo dritto 2 mm il piu ritentivo, festonato 0 mm il meno (3,25 N senza \
                     attachment); gli attachment aumentano la ritenzione, soprattutto con il bordo \
                     festonato 0 mm.",
    },
    Ref {
        key: "dasy2015",
        authors: "Dasy H, Dasy A, Asatrian G, Rozsa N, Lee HF, Kwak JH",
        year: 2015,
        title: "Effects of variable attachment shapes and aligner material on aligner retention",
        source: "Angle Orthod 85(6):934-940",
        doi: "10.2319/091014-637.1",
        pmid: "26516708",
        finding: "Senza attachment materiali piu duri o spessi non aumentano in modo significativo \
                     la ritenzione; gli attachment smussati (beveled) la aumentano in modo \
                     significativo, gli ellissoidali no.",
    },
    Ref {
        key: "bezzon1997",
        authors: "Bezzon OL, Mattos MG, Ribeiro RF",
        year: 1997,
        title: "Surveying removable partial dentures: the importance of guiding planes and path of \
                   insertion for stability",
        source: "J Prosthet Dent 78(4):412-418",
        doi: "10.1016/s0022-3913(97)70051-9",
        pmid: "9338875",
        finding: "Il parallelometro stabilisce l'asse di inserzione e rimozione e le zone di \
                     sottosquadro di un dispositivo rimovibile; avverte contro un asse perpendicolare \
                     al piano occlusale scelto senza analisi e contro inclinazioni estreme del modello.",
    },
    Ref {
        key: "wu2009",
        authors: "Wu L, Lu PJ, Wang Y, Ai HJ, Han J",
        year: 2009,
        title: "The identification of electronic surveying lines on 3-D digital models of dentition \
                   defects",
        source: "Zhonghua Kou Qiang Yi Xue Za Zhi 44(2):117-120 (in cinese)",
        doi: "",
        pmid: "19563048",
        finding: "Linea equatoriale digitale su modelli 3D: dai punti di massimo contorno rispetto a \
                     un asse di inserzione il modello e diviso in zone di sottosquadro e non; \
                     profondita del sottosquadro misurata automaticamente.",
    },
    Ref {
        key: "hahn2009",
        authors: "Hahn W, Dathe H, Fialka-Fricke J, Fricke-Zech S, Zapf A, Kubein-Meesenburg D, \
                     Sadat-Khonsari R",
        year: 2009,
        title: "Influence of thermoplastic appliance thickness on the magnitude of force delivered \
                   to a maxillary central incisor during tipping",
        source: "Am J Orthod Dentofacial Orthop 136(1):12.e1-12.e7",
        doi: "10.1016/j.ajodo.2008.12.015",
        pmid: "19577136",
        finding: "Erkodur 1,0 vs 0,8 mm e Biolon 1,0 vs 0,75 mm: le mascherine piu spesse danno \
                     forze significativamente maggiori (P<0,0001); forze 1,62-5,35 N, spesso sopra i \
                     valori ideali.",
    },
    Ref {
        key: "kohda2013",
        authors: "Kohda N, Iijima M, Muguruma T, Brantley WA, Ahluwalia KS, Mizoguchi I",
        year: 2013,
        title: "Effects of mechanical properties of thermoplastic materials on the initial force of \
                   thermoplastic appliances",
        source: "Angle Orthod 83(3):476-483",
        doi: "10.2319/052512-432.1",
        pmid: "23035832",
        finding: "Duran, Erkodur e Hardcast: le mascherine da 0,75-0,8 mm danno sempre forze \
                     significativamente maggiori di quelle da 0,4-0,5 mm; la forza dipende da \
                     materiale, spessore e attivazione.",
    },
    Ref {
        key: "kwon2008",
        authors: "Kwon JS, Lee YK, Lim BS, Lim YK",
        year: 2008,
        title: "Force delivery properties of thermoplastic orthodontic materials",
        source: "Am J Orthod Dentofacial Orthop 133(2):228-234",
        doi: "10.1016/j.ajodo.2006.03.034",
        pmid: "18249289",
        finding: "Flessione a tre punti di fogli da 0,508, 0,762 e 1,016 mm: nella deflessione \
                     ottimale (0,2-0,5 mm) il foglio sottile restituisce piu energia di quelli spessi \
                     dello stesso marchio (compromesso tra rigidezza e resilienza).",
    },
    Ref {
        key: "grant2023",
        authors: "Grant J, Foley P, Bankhead B, Miranda G, Adel SM, Kim KB",
        year: 2023,
        title: "Forces and moments generated by 3D direct printed clear aligners of varying labial \
                   and lingual thicknesses during lingual movement of maxillary central incisor: an in \
                   vitro study",
        source: "Prog Orthod 24(1):23",
        doi: "10.1186/s40510-023-00475-2",
        pmid: "37423974",
        finding: "Allineatori stampati (Tera Harz TC-85) da 0,50 mm con zone ispessite a 1,00 mm: lo \
                     spessore cambia forze e momenti, in modo complesso da prevedere.",
    },
    Ref {
        key: "bandic2024",
        authors: "Bandic R, Vodanovic K, Vukovic Kekez I, Medvedec Mikic I, Galic I, Kalibovic \
                     Govorko D",
        year: 2024,
        title: "Thickness Variations of Thermoformed and 3D-Printed Clear Aligners",
        source: "Acta Stomatol Croat 58(2):145-155",
        doi: "10.15644/asc58/2/4",
        pmid: "39036327",
        finding: "I termoformati risultano piu sottili del foglio di partenza, gli stampati per lo \
                     piu piu spessi del progetto; entrambi sono piu sottili ai bordi che su cuspidi e \
                     solchi.",
    },
    Ref {
        key: "timoshenko1959",
        authors: "Timoshenko SP, Woinowsky-Krieger S",
        year: 1959,
        title: "Theory of Plates and Shells, 2nd ed.",
        source: "McGraw-Hill, New York (ISBN 9780070647794)",
        doi: "",
        pmid: "",
        finding: "Rigidezza a flessione di una lamina D = E t^3 / [12 (1 - v^2)]: con il doppio \
                     dello spessore la rigidezza a flessione e circa 8 volte maggiore.",
    },
    Ref {
        key: "bucci2019",
        authors: "Bucci R, Rongo R, Levate C, Michelotti A, Barone S, Razionale AV, D'Anto V",
        year: 2019,
        title: "Thickness of orthodontic clear aligners after thermoforming and after 10 days of \
                   intraoral exposure: a prospective clinical study",
        source: "Prog Orthod 20(1):36",
        doi: "10.1186/s40510-019-0289-6",
        pmid: "31495908",
        finding: "Spessore dopo termoformatura riproducibile (errore max 0,13 mm).",
    },
    Ref {
        key: "edelmann2020",
        authors: "Edelmann A, English JD, Chen SJ, Kasper FK",
        year: 2020,
        title: "Analysis of the thickness of 3-dimensional-printed orthodontic aligners",
        source: "Am J Orthod Dentofacial Orthop 158(5):e91-e98",
        doi: "10.1016/j.ajodo.2020.07.029",
        pmid: "33131570",
        finding: "Allineatori progettati da 0,500, 0,750 e 1,000 mm stampati in SLA: tutti piu \
                     spessi del file di progetto (resina Dental LT +0,254, +0,267 e +0,274 mm).",
    },
    Ref {
        key: "koenig2022",
        authors: "Koenig N, Choi JY, McCray J, Hayes A, Schneider P, Kim KB",
        year: 2022,
        title: "Comparison of dimensional accuracy between direct-printed and thermoformed aligners",
        source: "Korean J Orthod 52(4):249-257",
        doi: "10.4041/kjod21.269",
        pmid: "35466087",
        finding: "Gli allineatori stampati direttamente sono piu accurati dei termoformati.",
    },
    Ref {
        key: "nucera2022",
        authors: "Nucera R, Dolci C, Bellocchio AM, Costa S, Barbera S, Rustico L, Farronato M, \
                     Militi A, Portelli M",
        year: 2022,
        title: "Effects of Composite Attachments on Orthodontic Clear Aligners Therapy: A Systematic \
                   Review",
        source: "Materials 15(2):533",
        doi: "10.3390/ma15020533",
        pmid: "35057250",
        finding: "Gli attachment migliorano torque radicolare anteriore, rotazione e movimento \
                     mesiodistale.",
    },
    Ref {
        key: "grunheid2017",
        authors: "Grunheid T, Loh C, Larson BE",
        year: 2017,
        title: "How accurate is Invisalign in nonextraction cases? Are predicted tooth positions \
                   achieved?",
        source: "Angle Orthod 87(6):809-815",
        doi: "10.2319/022717-147.1",
        pmid: "28686090",
        finding: "Sovrapposizione per best-fit di superficie; differenze clinicamente rilevanti \
                     oltre 0,5 mm e 2 gradi.",
    },
    Ref {
        key: "besl1992",
        authors: "Besl PJ, McKay ND",
        year: 1992,
        title: "A method for registration of 3-D shapes",
        source: "IEEE Trans Pattern Anal Mach Intell 14(2):239-256",
        doi: "10.1109/34.121791",
        pmid: "",
        finding: "Iterative Closest Point (ICP) per la registrazione rigida di superfici.",
    },
    Ref {
        key: "chetverikov2002",
        authors: "Chetverikov D, Svirko D, Stepanov D, Krsek P",
        year: 2002,
        title: "The Trimmed Iterative Closest Point algorithm",
        source: "Proc. 16th Int Conf Pattern Recognition 3:545-548",
        doi: "10.1109/ICPR.2002.1047997",
        pmid: "",
        finding: "ICP robusto ai valori anomali (minimi quadrati trimmed).",
    },
    Ref {
        key: "kabsch1976",
        authors: "Kabsch W",
        year: 1976,
        title: "A solution for the best rotation to relate two sets of vectors",
        source: "Acta Crystallogr A 32(5):922-923",
        doi: "10.1107/S0567739476001873",
        pmid: "",
        finding: "Rotazione ottima tra due insiemi di punti corrispondenti.",
    },
    Ref {
        key: "chen1992",
        authors: "Chen Y, Medioni G",
        year: 1992,
        title: "Object modelling by registration of multiple range images",
        source: "Image Vis Comput 10(3):145-155",
        doi: "10.1016/0262-8856(92)90066-C",
        pmid: "",
        finding: "ICP punto-piano: minimizza la distanza dal piano tangente, converge meglio sulle \
                     superfici che scorrono.",
    },
    Ref {
        key: "rusinkiewicz2001",
        authors: "Rusinkiewicz S, Levoy M",
        year: 2001,
        title: "Efficient variants of the ICP algorithm",
        source: "Proc. 3rd Int Conf 3-D Digital Imaging and Modeling (3DIM):145-152",
        doi: "10.1109/IM.2001.924423",
        pmid: "",
        finding: "Varianti di ICP: scarto delle coppie con normali incompatibili, metrica \
                     punto-piano.",
    },
    Ref {
        key: "lai2009",
        authors: "Lai YK, Hu SM, Martin RR, Rosin PL",
        year: 2009,
        title: "Rapid and effective segmentation of 3D models using random walks",
        source: "Comput Aided Geom Des 26(6):665-679",
        doi: "10.1016/j.cagd.2008.09.007",
        pmid: "",
        finding: "Random walk sulle facce: d = eta(1-cos(angolo diedro)), eta=1 concavo / 0,2 \
                     convesso, p = |e| exp(-d/sigma), sigma = 1; semplificazione a 10-20k facce.",
    },
    Ref {
        key: "zou2015",
        authors: "Zou BJ, Liu SJ, Liao SH, et al.",
        year: 2015,
        title: "Interactive tooth partition of dental mesh base on tooth-target harmonic field",
        source: "Comput Biol Med 56:132-144",
        doi: "10.1016/j.compbiomed.2014.10.013",
        pmid: "25464355",
        finding: "Segmentazione interattiva dei denti con campi armonici.",
    },
    Ref {
        key: "liao2015",
        authors: "Liao SH, Liu SJ, Zou BJ, et al.",
        year: 2015,
        title: "Automatic Tooth Segmentation of Dental Mesh Based on Harmonic Fields",
        source: "Biomed Res Int 2015:187173",
        doi: "10.1155/2015/187173",
        pmid: "26413507",
        finding: "Vincoli armonici con conoscenza a priori.",
    },
    Ref {
        key: "grady2006",
        authors: "Grady L",
        year: 2006,
        title: "Random walks for image segmentation",
        source: "IEEE Trans Pattern Anal Mach Intell 28(11):1768-1783",
        doi: "10.1109/TPAMI.2006.233",
        pmid: "",
        finding: "Formulazione del random walker come problema di Dirichlet.",
    },
    Ref {
        key: "shepard1968",
        authors: "Shepard D",
        year: 1968,
        title: "A two-dimensional interpolation function for irregularly-spaced data",
        source: "Proc. 23rd ACM National Conference:517-524",
        doi: "10.1145/800186.810616",
        pmid: "",
        finding: "Interpolazione a distanza inversa (deformazione della gengiva).",
    },
    Ref {
        key: "wendland1995",
        authors: "Wendland H",
        year: 1995,
        title: "Piecewise polynomial, positive definite and compactly supported radial functions of \
                   minimal degree",
        source: "Adv Comput Math 4:389-396",
        doi: "10.1007/BF02123482",
        pmid: "",
        finding: "Funzioni radiali a supporto compatto.",
    },
    Ref {
        key: "liepa2003",
        authors: "Liepa P",
        year: 2003,
        title: "Filling holes in meshes",
        source: "Eurographics Symposium on Geometry Processing:200-206",
        doi: "10.2312/SGP/SGP03/200-206",
        pmid: "",
        finding: "Chiusura dei buchi di una mesh con la superficie piu liscia raccordata ai bordi.",
    },
    Ref {
        key: "kobbelt1998",
        authors: "Kobbelt L, Campagna S, Vorsatz J, Seidel HP",
        year: 1998,
        title: "Interactive multi-resolution modeling on arbitrary meshes",
        source: "Proc. SIGGRAPH 98:105-114",
        doi: "10.1145/280814.280831",
        pmid: "",
        finding: "Superfici di energia minima (bi-laplaciano discreto) tangenti alla parte fissa \
                     della mesh.",
    },
    Ref {
        key: "joshi2007",
        authors: "Joshi P, Meyer M, DeRose T, Green B, Sanocki T",
        year: 2007,
        title: "Harmonic coordinates for character articulation",
        source: "ACM Trans Graph 26(3):71",
        doi: "10.1145/1276377.1276466",
        pmid: "",
        finding: "Pesi armonici: la deformazione interpola l'intera superficie senza pieghe ne \
                     strappi.",
    },
    Ref {
        key: "proffit2018",
        authors: "Proffit WR, Fields HW, Larson BE, Sarver DM",
        year: 2018,
        title: "Contemporary Orthodontics, 6th ed.",
        source: "Elsevier, Philadelphia (ISBN 9780323543873)",
        doi: "",
        pmid: "",
        finding: "Occlusione normale: classe I molare di Angle, overjet di circa 2-3 mm.",
    },
    Ref {
        key: "okeson2019",
        authors: "Okeson JP",
        year: 2019,
        title: "Management of Temporomandibular Disorders and Occlusion, 8th ed.",
        source: "Elsevier, St. Louis (ISBN 9780323582100)",
        doi: "",
        pmid: "",
        finding: "In massima intercuspidazione i contatti dei denti posteriori sono bilaterali e \
                     simultanei.",
    },
    Ref {
        key: "solaberrieta2015",
        authors: "Solaberrieta E, Otegi JR, Goicoechea N, Brizuela A, Pradies G",
        year: 2015,
        title: "Comparison of a conventional and virtual occlusal record",
        source: "J Prosthet Dent 114(1):92-97",
        doi: "10.1016/j.prosdent.2015.01.009",
        pmid: "",
        finding: "Registro occlusale virtuale da scansioni vestibolari: deviazione media 0,069 mm \
                     (DS 0,011).",
    },
    Ref {
        key: "solaberrieta2016",
        authors: "Solaberrieta E, Arias A, Brizuela A, Garikano X, Pradies G",
        year: 2016,
        title: "Determining the requirements, section quantity, and dimension of the virtual \
                   occlusal record",
        source: "J Prosthet Dent 115(1):52-56",
        doi: "10.1016/j.prosdent.2015.06.013",
        pmid: "",
        finding: "Registrazioni laterali destra + sinistra: miglior posizionamento della mandibola; \
                     dimensione minima efficace del registro 12 x 15 mm.",
    },
    Ref {
        key: "solaberrieta2016b",
        authors: "Solaberrieta E, Garmendia A, Brizuela A, Otegi JR, Pradies G, Szentpetery A",
        year: 2016,
        title: "Intraoral Digital Impressions for Virtual Occlusal Records: Section Quantity and \
                   Dimensions",
        source: "BioMed Res Int 2016:7173824",
        doi: "10.1155/2016/7173824",
        pmid: "",
        finding: "Servono almeno 2 sezioni, il piu distanti possibile; migliore: 2 laterali + 1 \
                     frontale, ciascuna larga 2 denti (24 x 15 mm).",
    },
    Ref {
        key: "revillaleon2023teeth",
        authors: "Revilla-Leon M, Alonso Perez-Barquero J, Zubizarreta-Macho A, Barmak AB, Att W, \
                     Kois JC",
        year: 2023,
        title: "Influence of the Number of Teeth and Location of the Virtual Occlusal Record on the \
                   Accuracy of the Maxillo-Mandibular Relationship Obtained by Using An Intraoral \
                   Scanner",
        source: "J Prosthodont 32(3):253-258",
        doi: "10.1111/jopr.13526",
        pmid: "",
        finding: "Registri vestibolari di 4 denti con la maggiore esattezza, di 2 denti con la \
                     minore.",
    },
    Ref {
        key: "revillaleon2023sr",
        authors: "Revilla-Leon M, Barmak AB, Tohme H, Yilmaz B, Kois JC, Gomez-Polo M",
        year: 2023,
        title: "Factors that influence the accuracy of maxillomandibular relationship at maximum \
                   intercuspation acquired by using intraoral scanners: A systematic review",
        source: "J Dent 138:104718",
        doi: "10.1016/j.jdent.2023.104718",
        pmid: "",
        finding: "29 studi: consigliato un registro bilaterale posteriore di almeno 4 denti (o \
                     bilaterale + frontale); 2 o piu denti mancanti riducono l'accuratezza.",
    },
    Ref {
        key: "revillaleon2024space",
        authors: "Revilla-Leon M, Gomez-Polo M, Zeitler JM, Barmak AB, Kois JC, Perez-Barquero JA",
        year: 2024,
        title: "Does the available interocclusal space influence the accuracy of the \
                   maxillomandibular relationship captured with an intraoral scanner?",
        source: "J Prosthet Dent 132(2):435-440",
        doi: "10.1016/j.prosdent.2022.09.004",
        pmid: "",
        finding: "In massima intercuspidazione distorsione 135 micrometri (esattezza) e 111 \
                     (precisione): la condizione peggiore tra quelle provate.",
    },
    Ref {
        key: "revillaleon2024collision",
        authors: "Revilla-Leon M, Gomez-Polo M, Barmak AB, Kois JC, Yilmaz B, Alonso Perez-Barquero J",
        year: 2024,
        title: "Influence of occlusal collision corrections completed by two intraoral scanners or a \
                   dental design program on the accuracy of the maxillomandibular relationship",
        source: "J Prosthet Dent 132(1):191-203",
        doi: "10.1016/j.prosdent.2023.05.015",
        pmid: "",
        finding: "Collisioni corrette dallo scanner: esattezza maggiore; collisioni lasciate e poi \
                     rifilate sulla mesh nel CAD: esattezza minore.",
    },
    Ref {
        key: "mei2022",
        authors: "Mei J, Ma L, Chao J, Liu F, Shen J",
        year: 2022,
        title: "Three-dimensional analysis of the outcome of different scanning strategies in \
                   virtual interocclusal registration",
        source: "J Adv Prosthodont 14(6):369-378",
        doi: "10.4047/jap.2022.14.6.369",
        pmid: "",
        finding: "La scansione bilaterale dei settori posteriori da la minore deviazione sull'intera \
                     arcata; quella di un solo lato e accurata solo vicino al lato scansionato.",
    },
    Ref {
        key: "iwauchi2022",
        authors: "Iwauchi Y, Tanaka S, Kamimura-Sugimura E, Baba K",
        year: 2022,
        title: "Clinical evaluation of the precision of interocclusal registration by using digital \
                   and conventional techniques",
        source: "J Prosthet Dent 128(4):611-617",
        doi: "10.1016/j.prosdent.2021.01.021",
        pmid: "",
        finding: "Precisione in vivo della registrazione digitale 25-31 micrometri (registrazioni \
                     convenzionali 128-154).",
    },
    Ref {
        key: "koshiishi2024",
        authors: "Koshiishi Y, Tanaka S, Iwauchi Y, Baba K",
        year: 2024,
        title: "Impact of scanning range and image count on the precision of digitally recorded \
                   intermaxillary relationships in interocclusal record using intraoral scanner",
        source: "J Oral Sci 66(2):111-115",
        doi: "10.2334/josnusd.23-0379",
        pmid: "",
        finding: "Discrepanza mediana 20-27 micrometri scansionando dal primo premolare al secondo \
                     molare, 30-42 limitando la scansione al primo molare.",
    },
    Ref {
        key: "garikano2024",
        authors: "Garikano X, Amezua X, Iturrate M, Solaberrieta E",
        year: 2024,
        title: "Evaluation of repeatability of different alignment methods to obtain digital \
                   interocclusal records: An in vitro study",
        source: "J Prosthet Dent 131(4):709-717",
        doi: "10.1016/j.prosdent.2022.07.014",
        pmid: "",
        finding: "Ripetibilita mediana: 47 micrometri (allineamento automatico dello scanner), 41 \
                     (best-fit globale nel CAD), 16 (best-fit su riferimenti).",
    },
    Ref {
        key: "kakali2023",
        authors: "Kakali L, Halazonetis DJ",
        year: 2023,
        title: "A novel method for testing accuracy of bite registration using intraoral scanners",
        source: "Korean J Orthod 53(4):254-263",
        doi: "10.4041/kjod22.199",
        pmid: "",
        finding: "Errore interocclusale massimo medio oltre 50 micrometri con tre scanner; errori di \
                     rollio o beccheggio oltre 0,1 gradi aggiungono circa 40 micrometri.",
    },
    Ref {
        key: "wong2018",
        authors: "Wong KY, Esguerra RJ, Chia VAP, Tan YH, Tan KBC",
        year: 2018,
        title: "Three-Dimensional Accuracy of Digital Static Interocclusal Registration by Three \
                   Intraoral Scanner Systems",
        source: "J Prosthodont 27(2):120-128",
        doi: "10.1111/jopr.12714",
        pmid: "",
        finding: "Distorsione 3D media della registrazione vestibolare da -579 a +70 micrometri \
                     secondo scanner e regione.",
    },
    Ref {
        key: "beck2023",
        authors: "Beck F, Lettner S, Zupancic Cepic L, Schedle A",
        year: 2023,
        title: "Comparison of Virtual Intersection and Occlusal Contacts between Intraoral and \
                   Laboratory Scans: An In-Vivo Study",
        source: "J Clin Med 12(3):996",
        doi: "10.3390/jcm12030996",
        pmid: "",
        finding: "Area di intersezione virtuale 48-79 mm2 con lo scanner intraorale (2-6 mm2 con lo \
                     scanner da laboratorio); contatto definito come distanza 0-10 o 0-100 micrometri.",
    },
    Ref {
        key: "zhao2026",
        authors: "Zhao X, Tu Y, Ding Q, Zhang L",
        year: 2026,
        title: "Effect of a digital bite registration method considering physiological tooth \
                   displacement on virtual occlusion: A self-controlled clinical trial",
        source: "J Prosthodont (online)",
        doi: "10.1111/jopr.70162",
        pmid: "",
        finding: "Registrazione automatica dello scanner: distanza interocclusale minima da -136 a \
                     -79 micrometri (compenetrazione); considerando lo spostamento fisiologico dei \
                     denti da -51 a +1.",
    },
    Ref {
        key: "lee2018contact",
        authors: "Lee H, Cha J, Chun YS, Kim M",
        year: 2018,
        title: "Comparison of the occlusal contact area of virtual models and actual models: a \
                   comparative in vitro study on Class I and Class II malocclusion models",
        source: "BMC Oral Health 18(1):109",
        doi: "10.1186/s12903-018-0566-7",
        pmid: "",
        finding: "Contatto = distanza 0-0,098 mm della scala di colore; accordo con Prescale nei \
                     settori posteriori, contatti anteriori sovrastimati.",
    },
    Ref {
        key: "edher2018",
        authors: "Edher F, Hannam AG, Tobias DL, Wyatt CCL",
        year: 2018,
        title: "The accuracy of virtual interocclusal registration during intraoral scanning",
        source: "J Prosthet Dent 120(6):904-912",
        doi: "10.1016/j.prosdent.2018.01.024",
        pmid: "",
        finding: "Contatti virtuali dello scanner: sensibilita 92,9%, valore predittivo negativo \
                     84,2%.",
    },
    Ref {
        key: "popa2024",
        authors: "Popa AD, Vladutu DE, Turcu AA, Tartea DA, Ionescu M, Paunescu C, Stan RS, Mercut V",
        year: 2024,
        title: "Aspects of Occlusal Recordings Performed with the T-Scan System and with the Medit \
                   Intraoral Scanner",
        source: "Diagnostics 14(13):1457",
        doi: "10.3390/diagnostics14131457",
        pmid: "",
        finding: "Mappa occlusale Medit: rosso = sovrapposizione oltre 0,1 mm, verde = contatto, blu \
                     = contatto leggero.",
    },
    Ref {
        key: "vladutu2025",
        authors: "Vladutu DE, Mercut R, Varut MC, Stefarta A, Mercut V, Radoi AM, Bratoiu MR, Popa \
                     AD, Popescu AM, Dica S, Stan RS, Tartea DA",
        year: 2025,
        title: "Assessment of Occlusal Contacts Recorded with the Medit Intraoral Scanner vs. Exocad \
                   Software",
        source: "J Clin Med 14(20):7378",
        doi: "10.3390/jcm14207378",
        pmid: "",
        finding: "Scala Medit dal blu al rosso (rosso = compenetrazione); scala exocad magenta \
                     (collisione) - verde - blu.",
    },
    Ref {
        key: "utt1995",
        authors: "Utt TW, Meyers CE, Wierzba TF, Hondrum SO",
        year: 1995,
        title: "A three-dimensional comparison of condylar position changes between centric relation \
                   and centric occlusion using the mandibular position indicator",
        source: "Am J Orthod Dentofacial Orthop 107(3):298-308",
        doi: "10.1016/S0889-5406(95)70146-X",
        pmid: "",
        finding: "107 pazienti ortodontici: solo lo 0,9% senza differenza tra relazione centrica e \
                     occlusione centrica; spostamento condilare medio 0,84 mm verticale e 0,61 mm \
                     antero-posteriore; 18,7% almeno 2 mm.",
    },
    Ref {
        key: "gpt9",
        authors: "The Academy of Prosthodontics",
        year: 2017,
        title: "The Glossary of Prosthodontic Terms: Ninth Edition",
        source: "J Prosthet Dent 117(5S):e1-e105",
        doi: "10.1016/j.prosdent.2016.12.001",
        pmid: "28418832",
        finding: "Massima intercuspidazione: intercuspidazione completa dei denti indipendente dalla \
                     posizione dei condili; relazione centrica: rapporto mascello-mandibolare \
                     indipendente dal contatto dei denti. Equatore (height of contour, survey line): \
                     linea che circonda il dente e ne indica la massima circonferenza rispetto all'asse \
                     di inserzione scelto; sottosquadro: parte della superficie sotto l'equatore \
                     rispetto all'asse di inserzione.",
    },
    Ref {
        key: "chetverikov2005",
        authors: "Chetverikov D, Stepanov D, Krsek P",
        year: 2005,
        title: "Robust Euclidean alignment of 3D point sets: the trimmed iterative closest point \
                   algorithm",
        source: "Image Vis Comput 23(3):299-309",
        doi: "10.1016/j.imavis.2004.05.007",
        pmid: "",
        finding: "Trimmed ICP con sovrapposizione stimata automaticamente (minimo di \
                     e(xi)/xi^(1+lambda)); funziona con sovrapposizioni sotto il 50%.",
    },
    Ref {
        key: "wriedt2013",
        authors: "Wriedt S, Schmidtmann I, Niemann M, Wehrbein H",
        year: 2013,
        title: "Digital 3D image of bimaxillary casts connected by a vestibular scan",
        source: "J Orofac Orthop 74(4):309-318",
        doi: "10.1007/s00056-013-0152-1",
        pmid: "",
        finding: "Modelli in gesso delle due arcate uniti da una scansione vestibolare: deviazione \
                     massima 37 +/- 28 micrometri.",
    },
    Ref {
        key: "auskalnis2024",
        authors: "Auskalnis L, Akulauskas M, Osnes C, Revilla-Leon M, Kernen-Gintaute A, Rutkunas V",
        year: 2024,
        title: "Trueness of maxillomandibular relationship in 3D-printed and conventional casts",
        source: "J Dent 148:105044",
        doi: "10.1016/j.jdent.2024.105044",
        pmid: "",
        finding: "La relazione registrata con lo scanner intraorale e esatta quanto o piu di quella \
                     dei modelli in gesso o stampati montati in occlusione.",
    },
    Ref {
        key: "3shape2013",
        authors: "3Shape A/S",
        year: 2013,
        title: "TRIOS User Manual (TRIOS-2013-1-1.2.1.1-B-EN)",
        source: "3Shape, Copenaghen",
        doi: "",
        pmid: "",
        finding: "Morso 1 sempre; morso 2 per l'arcata completa, sul lato opposto; scansionare 4 \
                     denti per un allineamento ottimale; allineamento automatico, manuale con punti su \
                     arcate e morso.",
    },
    Ref {
        key: "3shape2018",
        authors: "3Shape A/S",
        year: 2018,
        title: "TRIOS info pack: Bite Alignment (v1.1)",
        source: "3Shape, Copenaghen",
        doi: "",
        pmid: "",
        finding: "Arcate scansionate scariche, morso sotto carico: il legamento parodontale lascia \
                     spostare i denti e crea compenetrazioni con ogni scanner. La funzione Adjust trova \
                     l'occlusione piu vicina ai morsi senza compenetrazioni allontanando le arcate; non \
                     porta a contatto arcate che non si toccano.",
    },
    Ref {
        key: "dentsply2019",
        authors: "Dentsply Sirona",
        year: 2019,
        title: "Connect SW 5.1.x Operator's Manual",
        source: "Dentsply Sirona, Bensheim",
        doi: "",
        pmid: "",
        finding: "Registrazione vestibolare destra e sinistra (premolari-molari, denti delle due \
                     arcate e 5 mm di gengiva); allineamento automatico, manuale trascinando; contatti \
                     in classi di 50 micrometri (compenetrazione >100, 100-50, 50-0; distanza 0-50, \
                     50-100, >100).",
    },
    Ref {
        key: "itero2020",
        authors: "Align Technology",
        year: 2020,
        title: "iTero Full-Arch Scanning Guide (MKT-0004794 Rev A); iTero Element 5D User Manual",
        source: "Align Technology, San Jose",
        doi: "",
        pmid: "",
        finding: "Morso in occlusione centrica, 3-4 denti per lato, ripetuto sull'altro lato; avvisi \
                     per morso di un solo lato e per discordanza tra i morsi destro e sinistro.",
    },
    Ref {
        key: "museth2013",
        authors: "Museth K",
        year: 2013,
        title: "VDB: High-resolution sparse volumes with dynamic topology",
        source: "ACM Trans Graph 32(3):27",
        doi: "10.1145/2487228.2487235",
        pmid: "",
        finding: "Volumi sparsi (OpenVDB): ricostruzione a voxel di una superficie chiusa senza \
                     auto-intersezioni.",
    },
    Ref {
        key: "szilvasinagy2003",
        authors: "Szilvasi-Nagy M, Matyasi G",
        year: 2003,
        title: "Analysis of STL files",
        source: "Math Comput Model 38(7-9):945-960",
        doi: "10.1016/S0895-7177(03)90079-3",
        pmid: "",
        finding: "Difetti degli STL (buchi, spigoli non manifold, normali incoerenti, \
                     sovrapposizioni) e loro verifica.",
    },
    Ref {
        key: "moller1997",
        authors: "Moller T",
        year: 1997,
        title: "A fast triangle-triangle intersection test",
        source: "J Graph Tools 2(2):25-30",
        doi: "10.1080/10867651.1997.10487472",
        pmid: "",
        finding: "Test di intersezione tra triangoli, base della ricerca delle auto-intersezioni.",
    },
];

/// Every evidence-based default, in the plugin's order.
pub static PARAMS: &[Param] = &[
    Param {
        key: "step_linear",
        value: ParamValue::Float(0.25),
        unit: "mm",
        refs: &[
            "castroflorio2023",
            "simon2014",
            "wang2023",
            "martinezlozano2024",
        ],
        note: "Traslazione massima per allineatore (mesiodistale, vestibololinguale)",
        to_confirm: false,
    },
    Param {
        key: "step_vertical",
        value: ParamValue::Float(0.25),
        unit: "mm",
        refs: &["castroflorio2023", "martinezlozano2024"],
        note: "Intrusione/estrusione massima per allineatore (movimenti lineari)",
        to_confirm: false,
    },
    Param {
        key: "step_rotation",
        value: ParamValue::Float(2.0),
        unit: "deg",
        refs: &["castroflorio2023", "simon2014"],
        note: "Rotazione massima per allineatore (incisivi e molari)",
        to_confirm: false,
    },
    Param {
        key: "step_rotation_round",
        value: ParamValue::Float(1.5),
        unit: "deg",
        refs: &["simon2014", "castroflorio2023"],
        note: "Rotazione massima per allineatore per denti a corona tonda (canini, premolari): sopra \
                  1,5 gradi l'accuratezza scende dal 41,8% al 23,2%",
        to_confirm: false,
    },
    Param {
        key: "step_angular",
        value: ParamValue::Float(1.0),
        unit: "deg",
        refs: &["castroflorio2023", "simon2014"],
        note: "Torque e tip (movimenti angolari) massimi per allineatore",
        to_confirm: false,
    },
    Param {
        key: "max_crown_step",
        value: ParamValue::Float(0.33),
        unit: "mm",
        refs: &["wang2023", "yang2025"],
        note: "Spostamento massimo di qualsiasi punto della corona per allineatore (limite \
                  complessivo, intervallo suggerito 0,25-0,33 mm)",
        to_confirm: false,
    },
    Param {
        key: "perio_step",
        value: ParamValue::Map(&[
            ("HEALTHY", 0.25),
            ("STAGE1", 0.18),
            ("STAGE2", 0.15),
            ("STAGE3", 0.1),
        ]),
        unit: "mm",
        refs: &["mali2021"],
        note: "Passo lineare per condizione parodontale (sano, stadio I, II, III)",
        to_confirm: false,
    },
    Param {
        key: "interval_days",
        value: ParamValue::Int(14),
        unit: "days",
        refs: &["bollen2003", "castroflorio2023", "alnadawi2021"],
        note: "Giorni per allineatore",
        to_confirm: false,
    },
    Param {
        key: "wear_hours",
        value: ParamValue::Int(22),
        unit: "h/day",
        refs: &["timm2021"],
        note: "Ore di utilizzo giornaliero richieste",
        to_confirm: false,
    },
    Param {
        key: "seq_overlap",
        value: ParamValue::Float(0.5),
        unit: "",
        refs: &["martinezlozano2024", "ravera2016"],
        note: "Distalizzazione sequenziale 1/2: il dente successivo parte a meta movimento del \
                  precedente",
        to_confirm: false,
    },
    Param {
        key: "collision_tol",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &["martinezlozano2024", "liu2025pf", "yang2025"],
        note: "Sovrapposizione tollerata tra denti adiacenti durante lo staging",
        to_confirm: false,
    },
    Param {
        key: "expansion_upper",
        value: ParamValue::Float(2.0),
        unit: "mm/side",
        refs: &["wang2025consensus", "houle2017", "zhou2020"],
        note: "Espansione massima per lato (arcata superiore)",
        to_confirm: false,
    },
    Param {
        key: "expansion_lower_posterior",
        value: ParamValue::Float(2.0),
        unit: "mm/side",
        refs: &["wang2025consensus"],
        note: "Espansione massima per lato dei settori posteriori inferiori",
        to_confirm: false,
    },
    Param {
        key: "expansion_lower_canine",
        value: ParamValue::Float(0.0),
        unit: "mm/side",
        refs: &["burke1998"],
        note: "Espansione intercanina inferiore (tende a recidivare)",
        to_confirm: false,
    },
    Param {
        key: "advance_incisors",
        value: ParamValue::Float(1.0),
        unit: "mm",
        refs: &[
            "wang2025consensus",
            "germane1991",
            "yared2006",
            "artun1987",
            "handelman1996",
        ],
        note: "Avanzamento massimo degli incisivi (1 mm circa = 2 mm di spazio)",
        to_confirm: false,
    },
    Param {
        key: "retract_incisors",
        value: ParamValue::Float(2.0),
        unit: "mm",
        refs: &["handelman1996"],
        note: "Retrazione massima degli incisivi per chiudere diastemi",
        to_confirm: false,
    },
    Param {
        key: "ipr_surface",
        value: ParamValue::Float(0.25),
        unit: "mm",
        refs: &["wang2025consensus", "zachrisson2007"],
        note: "IPR massima per superficie prossimale",
        to_confirm: false,
    },
    Param {
        key: "ipr_enamel_fraction",
        value: ParamValue::Float(0.5),
        unit: "",
        refs: &["wang2025consensus", "firincioglullari2025"],
        note: "Frazione massima dello spessore dello smalto rimovibile",
        to_confirm: false,
    },
    Param {
        key: "ipr_site_warning",
        value: ParamValue::Float(0.2),
        unit: "mm",
        refs: &["gonzalezgarcia2025"],
        note: "Soglia di avviso per IPR in un singolo sito",
        to_confirm: false,
    },
    Param {
        key: "extraction_threshold",
        value: ParamValue::Float(8.0),
        unit: "mm",
        refs: &["wang2025consensus"],
        note: "Discrepanza oltre la quale l'estrazione e indicata",
        to_confirm: false,
    },
    Param {
        key: "distal_predictable",
        value: ParamValue::Float(2.0),
        unit: "mm",
        refs: &["wang2025consensus", "ravera2016"],
        note: "Distalizzazione molare prevedibile per lato",
        to_confirm: false,
    },
    Param {
        key: "vertical_auto_max",
        value: ParamValue::Float(1.0),
        unit: "mm",
        refs: &["kravitz2009", "haouili2020", "martinezlozano2024"],
        note: "Correzione verticale automatica massima (movimenti verticali poco prevedibili)",
        to_confirm: false,
    },
    Param {
        key: "cons_line_sigma",
        value: ParamValue::Float(5.0),
        unit: "mm",
        refs: &[],
        note: "Lisciatura della linea d'arcata dalle cuspidi reali (scala lungo l'arcata): piu \
                  piccola segue di piu i singoli denti",
        to_confirm: true,
    },
    Param {
        key: "cons_normal_tol",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &[],
        note: "Un dente le cui cuspidi distano meno di cosi dalla linea d'arcata non ci viene \
                  portato (sotto la precisione di un allineatore)",
        to_confirm: true,
    },
    Param {
        key: "cons_rot_tol",
        value: ParamValue::Float(1.0),
        unit: "gradi",
        refs: &[],
        note: "Rotazione sotto la quale un dente non viene derotato",
        to_confirm: true,
    },
    Param {
        key: "cons_pos_tol",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &[],
        note: "Spostamento lungo l'arcata sotto il quale un dente resta fermo",
        to_confirm: true,
    },
    Param {
        key: "cons_contact_tol",
        value: ParamValue::Float(0.15),
        unit: "mm",
        refs: &[],
        note: "Sovrapposizione tra denti vicini lasciata com'e (contatto stretto o rumore della \
                  scansione); oltre e affollamento",
        to_confirm: true,
    },
    Param {
        key: "cons_incisor_weight",
        value: ParamValue::Float(4.0),
        unit: "",
        refs: &[],
        note: "Peso degli incisivi centrali nella ridistribuzione dello spazio (la linea mediana si \
                  sposta il meno possibile)",
        to_confirm: true,
    },
    Param {
        key: "cons_post_normal_tol",
        value: ParamValue::Float(0.3),
        unit: "mm",
        refs: &[],
        note: "Premolari e molari: distanza delle cuspidi dalla linea d'arcata sotto la quale il \
                  dente resta in posizione (la valutazione non lo sposta sulla linea)",
        to_confirm: true,
    },
    Param {
        key: "cons_post_rot_tol",
        value: ParamValue::Float(5.0),
        unit: "gradi",
        refs: &[],
        note: "Premolari e molari: rotazione rispetto alla linea d'arcata sotto la quale il dente \
                  resta in posizione (non viene derotato)",
        to_confirm: true,
    },
    Param {
        key: "cons_level_tol",
        value: ParamValue::Float(0.5),
        unit: "mm",
        refs: &[],
        note: "Gradino verticale tollerato tra un dente e la quota dei vicini: sotto il dente resta \
                  alla sua altezza, oltre si corregge solo l'eccesso (il dente al bordo della \
                  tolleranza, non sulla linea: movimenti verticali poco prevedibili, differenze \
                  naturali tra i denti)",
        to_confirm: true,
    },
    Param {
        key: "stamp_height",
        value: ParamValue::Float(0.7),
        unit: "mm",
        refs: &[],
        note: "Altezza dei caratteri della scritta (numero e data) dentro l'allineatore: piccola, i \
                  tratti restano di 2-4 volte la precisione della stampante (0,02-0,05 mm)",
        to_confirm: true,
    },
    Param {
        key: "stamp_depth",
        value: ParamValue::Float(0.08),
        unit: "mm",
        refs: &[],
        note: "Profondita della scritta incisa nel guscio dal lato del dente (sul modello per \
                  termoformatura: altezza del rilievo)",
        to_confirm: true,
    },
    Param {
        key: "stamp_min_wall",
        value: ParamValue::Float(0.35),
        unit: "mm",
        refs: &[],
        note: "Parete minima del guscio che resta sotto la scritta: dove la parete e piu sottile la \
                  scritta non viene incisa",
        to_confirm: true,
    },
    Param {
        key: "overcorrection",
        value: ParamValue::Map(&[
            ("torque", 0.205),
            ("tip", 0.145),
            ("rotation", 0.284),
            ("extrusion", 0.117),
            ("intrusion", 0.22),
        ]),
        unit: "",
        refs: &["palone2023"],
        note: "Ipercorrezione per tipo di movimento",
        to_confirm: false,
    },
    Param {
        key: "tracking_linear",
        value: ParamValue::Float(0.5),
        unit: "mm",
        refs: &["alnadawi2021", "grunheid2017"],
        note: "Scostamento lineare oltre il quale il dente non e in pari con il piano",
        to_confirm: false,
    },
    Param {
        key: "tracking_angular",
        value: ParamValue::Float(2.0),
        unit: "deg",
        refs: &["alnadawi2021", "grunheid2017"],
        note: "Scostamento angolare oltre il quale il dente non e in pari con il piano",
        to_confirm: false,
    },
    Param {
        key: "lag_speed",
        value: ParamValue::Float(0.75),
        unit: "",
        refs: &["simon2014"],
        note: "Velocita dei denti non in pari nei nuovi allineatori (staging piu lento = efficacia \
                  maggiore: 41,8% sotto 1,5 gradi/allineatore vs 23,2% sopra)",
        to_confirm: false,
    },
    Param {
        key: "cres_ratio",
        value: ParamValue::Float(0.42),
        unit: "",
        refs: &["poppe2002"],
        note: "Centro di resistenza: frazione dell'altezza alveolare dalla cresta",
        to_confirm: false,
    },
    Param {
        key: "margin_to_crest",
        value: ParamValue::Float(2.73),
        unit: "mm",
        refs: &["gargiulo1961"],
        note: "Distanza margine gengivale - cresta alveolare",
        to_confirm: false,
    },
    Param {
        key: "neck_depth",
        value: ParamValue::Float(1.66),
        unit: "mm",
        refs: &["gargiulo1961"],
        note: "Colletto del dente ricostruito sotto il margine gengivale (solco 0,69 + attacco \
                  epiteliale 0,97 mm)",
        to_confirm: false,
    },
    Param {
        key: "trim_extension",
        value: ParamValue::Float(2.0),
        unit: "mm",
        refs: &["elshazly2022", "elshazly2023", "traversa2025", "cowley2012"],
        note: "Estensione del bordo oltre il margine gengivale",
        to_confirm: false,
    },
    Param {
        key: "shell_thickness",
        value: ParamValue::Float(1.0),
        unit: "mm",
        refs: &[
            "hahn2009",
            "kohda2013",
            "timoshenko1959",
            "cowley2012",
            "kwon2008",
            "edelmann2020",
            "bandic2024",
            "koenig2022",
            "bucci2019",
        ],
        note: "Spessore di progetto del guscio (stampa diretta): 1,0 mm (fino al 2026-10-14 0,5 mm), \
                  come i fogli termoformati degli studi, da 0,75-1,0 mm. A parita di materiale la forza \
                  cresce con lo spessore e la rigidezza a flessione circa con il cubo (spessore doppio, \
                  circa 8 volte piu rigido); lo stampato puo risultare fino a circa 0,27 mm piu spesso \
                  del progetto",
        to_confirm: false,
    },
    Param {
        key: "trim_equator_offset",
        value: ParamValue::Float(0.0),
        unit: "mm",
        refs: &[
            "gpt9",
            "bezzon1997",
            "wu2009",
            "cowley2012",
            "nakornnoi2024",
            "bortoli2026",
            "tran2026",
            "dasy2015",
        ],
        note: "Bordo dell'allineatore rispetto all'equatore del dente (linea di massima convessita \
                  lungo la via di inserzione del dente): 0 = sull'equatore, nessun sottosquadro \
                  impegnato, inserimento e rimozione senza forzare; positivo = oltre l'equatore verso \
                  la gengiva (nel sottosquadro: piu ritenzione, inserimento piu difficile; dove \
                  l'equatore e sul margine gengivale il bordo resta sul margine); negativo = verso \
                  occlusale, al massimo fino a 1 mm dalla cima della corona. Lo spostamento vale su \
                  tutto il giro del dente: verso occlusale il bordo sale di quasi tutto il valore anche \
                  sul lato palatale o linguale; verso la gengiva scende fino al valore solo dove la \
                  gengiva sotto l'equatore lascia spazio, perche il guscio non copre mai la gengiva: \
                  dove e subito sotto (spesso premolari e molari inferiori, lato linguale, palato) \
                  scende poco o nulla (con +0,5 mm, sulle arcate reali di prova, il bordo finisce tra \
                  circa 0,2 mm sopra e 0,5 mm oltre l'equatore, mediana dei lati vestibolare e linguale \
                  di ogni dente; su premolari e molari spesso resta fermo all'equatore) e il pannello \
                  elenca i denti dove l'offset resta limitato; tra i denti il bordo resta dove serve \
                  per la giunzione. I bordi estesi oltre il margine gengivale sono i piu ritentivi: con \
                  il bordo all'equatore la ritenzione dipende dagli attachment",
        to_confirm: false,
    },
    Param {
        key: "insertion_tilt_anterior",
        value: ParamValue::Float(30.0),
        unit: "deg",
        refs: &["bezzon1997", "gpt9"],
        note: "Asse di ogni dente: inclinazione massima dell'asse di inserzione di un incisivo o \
                  canino rispetto all'asse dell'arcata. Come sul parallelometro l'asse si inclina solo \
                  dove lascia meno corona in sottosquadro (incisivi proinclinati); limite di progetto, \
                  gli studi non danno un valore. Con incisivi proinclinati il guscio non ha un'unica \
                  via rigida e per toglierlo deve flettere: il pannello lo segnala e propone l'asse \
                  comune",
        to_confirm: false,
    },
    Param {
        key: "insertion_tilt_posterior",
        value: ParamValue::Float(10.0),
        unit: "deg",
        refs: &["bezzon1997", "gpt9"],
        note: "Asse di ogni dente: inclinazione massima dell'asse di inserzione di un premolare o \
                  molare (i denti posteriori guidano l'inserzione dell'allineatore); limite di progetto",
        to_confirm: false,
    },
    Param {
        key: "insertion_tilt_neighbour",
        value: ParamValue::Float(10.0),
        unit: "deg",
        refs: &["bezzon1997", "gpt9"],
        note: "Asse di ogni dente: differenza massima tra gli assi di inserzione di due denti \
                  vicini. L'allineatore e un pezzo unico: assi molto diversi tra denti vicini lo \
                  costringono a flettersi tra loro e lo bloccano se tolto in un solo movimento; limite \
                  di progetto",
        to_confirm: false,
    },
    Param {
        key: "insertion_tilt_common",
        value: ParamValue::Float(20.0),
        unit: "deg",
        refs: &["bezzon1997", "gpt9"],
        note: "Asse comune ottimizzato: inclinazione massima dell'unico asse rigido dell'arcata \
                  rispetto alla perpendicolare al piano occlusale. Il parallelometro inclina il modello \
                  per ridurre i sottosquadri di tutti i denti ma avverte contro inclinazioni estreme; \
                  limite di progetto. Quando l'asse arriva al limite il pannello lo segnala: gli \
                  incisivi inclinati restano meno coperti che con l'asse di ogni dente",
        to_confirm: false,
    },
    Param {
        key: "occlusion_overjet",
        value: ParamValue::Float(2.5),
        unit: "mm",
        refs: &["proffit2018"],
        note: "Overjet della posizione di partenza (incisivi superiori davanti agli inferiori)",
        to_confirm: false,
    },
    Param {
        key: "occlusion_class_i",
        value: ParamValue::Float(2.5),
        unit: "mm",
        refs: &["proffit2018", "andrews1972", "wheeler"],
        note: "Primo molare superiore distale all'inferiore (classe I: cuspide mesiovestibolare nel \
                  solco vestibolare)",
        to_confirm: false,
    },
    Param {
        key: "occlusion_bite_warn",
        value: ParamValue::Float(0.5),
        unit: "mm",
        refs: &["casko1998"],
        note: "Variazione verticale del morso durante il trattamento oltre la quale si segnala un \
                  precontatto da verificare (ABO-OGS: contatti occlusali 0,5-1 mm = 1 punto)",
        to_confirm: false,
    },
    Param {
        key: "occlusal_thickness",
        value: ParamValue::Float(0.75),
        unit: "mm",
        refs: &[],
        note: "Spessore della parete sulle superfici occlusali quando e scelto a mano (le pareti \
                  assiali restano dello spessore del guscio): come i fogli da 0,75 mm",
        to_confirm: true,
    },
    Param {
        key: "occlusal_min_thickness",
        value: ParamValue::Float(0.5),
        unit: "mm",
        refs: &[],
        note: "Spessore occlusale minimo (calcolo automatico e molatura non scendono sotto): \
                  resistenza a masticazione e stampa",
        to_confirm: true,
    },
    Param {
        key: "bite_opening_target",
        value: ParamValue::Float(1.5),
        unit: "mm",
        refs: &[],
        note: "Apertura del morso ai primi molari causata dagli allineatori indossati che lo \
                  spessore occlusale automatico non supera (entro lo spazio libero di riposo di 2-4 mm, \
                  per limitare l'effetto bite-block sui posteriori)",
        to_confirm: true,
    },
    Param {
        key: "hinge_posterior",
        value: ParamValue::Float(78.0),
        unit: "mm",
        refs: &[],
        note: "Distanza dell'asse cerniera dei condili dietro il punto incisivo inferiore, lungo il \
                  piano occlusale (triangolo di Bonwill di 100 mm, angolo di Balkwill 26 gradi)",
        to_confirm: true,
    },
    Param {
        key: "hinge_height",
        value: ParamValue::Float(38.0),
        unit: "mm",
        refs: &[],
        note: "Altezza dell'asse cerniera dei condili sopra il piano occlusale (triangolo di Bonwill \
                  di 100 mm, angolo di Balkwill 26 gradi)",
        to_confirm: true,
    },
    Param {
        key: "equilibrate_teeth_per_side",
        value: ParamValue::Int(2),
        unit: "denti",
        refs: &[],
        note: "Denti in contatto per lato a cui si ferma l'equilibratura (contatti bilaterali \
                  simultanei)",
        to_confirm: true,
    },
    Param {
        key: "equilibrate_max",
        value: ParamValue::Float(0.3),
        unit: "mm",
        refs: &[],
        note: "Molatura massima di ciascun allineatore nell'equilibratura",
        to_confirm: true,
    },
    Param {
        key: "ramp_width",
        value: ParamValue::Float(3.0),
        unit: "mm",
        refs: &[],
        note: "Larghezza di una rampa nuova (lungo il dente)",
        to_confirm: true,
    },
    Param {
        key: "ramp_depth",
        value: ParamValue::Float(2.0),
        unit: "mm",
        refs: &[],
        note: "Profondita di una rampa nuova (verso la gengiva)",
        to_confirm: true,
    },
    Param {
        key: "ramp_height",
        value: ParamValue::Float(1.5),
        unit: "mm",
        refs: &[],
        note: "Altezza di una rampa nuova sulla superficie dell'allineatore: l'apertura posteriore \
                  che da nel morso profondo",
        to_confirm: true,
    },
    Param {
        key: "ramp_overbite_min",
        value: ParamValue::Float(4.0),
        unit: "mm",
        refs: &[],
        note: "Overbite degli incisivi oltre il quale le rampe anteriori sono proposte (morso \
                  profondo)",
        to_confirm: true,
    },
    Param {
        key: "bite_inlier",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &["iwauchi2022", "koshiishi2024", "garikano2024"],
        note: "Distanza entro cui un punto del morso giace sulla superficie dell'arcata (2-3 volte \
                  la precisione della registrazione con scanner intraorale, 20-47 micrometri)",
        to_confirm: false,
    },
    Param {
        key: "bite_rms_good",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &["iwauchi2022", "koshiishi2024", "kakali2023"],
        note: "Scarto RMS delle superfici sovrapposte di una registrazione buona (attesi 20-50 \
                  micrometri; oltre 0,10 mm da verificare)",
        to_confirm: false,
    },
    Param {
        key: "bite_rms_max",
        value: ParamValue::Float(0.15),
        unit: "mm",
        refs: &["kakali2023", "wong2018"],
        note: "Scarto RMS oltre il quale il morso non combacia con l'arcata e viene rifiutato \
                  (misurato su 4 pazienti Teeth3DS+: morso giusto 0,021-0,022 mm, morso di un altro \
                  paziente 0,16-0,24 mm in 24 prove)",
        to_confirm: false,
    },
    Param {
        key: "bite_coherence",
        value: ParamValue::Float(0.7),
        unit: "",
        refs: &["chetverikov2005"],
        note: "Quota minima delle superfici affacciate (entro 0,5 mm) che coincidono entro la \
                  tolleranza (misurata: morso giusto 0,999-1,0, morso di un altro paziente 0,26-0,51)",
        to_confirm: false,
    },
    Param {
        key: "bite_min_teeth",
        value: ParamValue::Int(2),
        unit: "denti",
        refs: &["solaberrieta2016b", "revillaleon2023teeth"],
        note: "Denti di ogni arcata che devono sovrapporsi al morso (sotto: rifiutato)",
        to_confirm: false,
    },
    Param {
        key: "bite_teeth",
        value: ParamValue::Int(4),
        unit: "denti",
        refs: &["revillaleon2023teeth", "revillaleon2023sr", "3shape2013"],
        note: "Denti consigliati in ogni scansione del morso (per lato)",
        to_confirm: false,
    },
    Param {
        key: "bite_min_area",
        value: ParamValue::Float(90.0),
        unit: "mm2",
        refs: &["solaberrieta2016"],
        note: "Superficie sovrapposta minima di ogni arcata (registro minimo efficace 12 x 15 mm per \
                  le due arcate insieme)",
        to_confirm: false,
    },
    Param {
        key: "bite_tooth_area",
        value: ParamValue::Float(8.0),
        unit: "mm2",
        refs: &["wheeler"],
        note: "Superficie sovrapposta perche un dente conti nel morso (circa 1/6 della faccia \
                  vestibolare di un premolare)",
        to_confirm: false,
    },
    Param {
        key: "bite_penetration_max",
        value: ParamValue::Float(1.0),
        unit: "mm",
        refs: &["wong2018", "zhao2026"],
        note: "Compenetrazione oltre la quale la relazione registrata non e plausibile (oltre il \
                  doppio della distorsione massima riportata, 0,58 mm)",
        to_confirm: false,
    },
    Param {
        key: "bite_penetration_warn",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &["popa2024", "zhao2026", "3shape2018"],
        note: "Compenetrazione da rivedere dopo la registrazione (correzione facoltativa: apertura \
                  rigida del morso)",
        to_confirm: false,
    },
    Param {
        key: "bite_gap_warn",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &["lee2018contact", "beck2023", "3shape2018"],
        note: "Distanza minima tra le arcate registrate oltre la quale non sono in contatto",
        to_confirm: false,
    },
    Param {
        key: "bite_agreement",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &["itero2020", "mei2022", "kakali2023"],
        note: "Differenza massima tra le relazioni dei due morsi (destro e sinistro)",
        to_confirm: false,
    },
    Param {
        key: "contact_map_contact",
        value: ParamValue::Float(0.1),
        unit: "mm",
        refs: &["lee2018contact", "beck2023", "dentsply2019"],
        note: "Distanza dall'antagonista entro cui un punto e in contatto occlusale",
        to_confirm: false,
    },
    Param {
        key: "rw_eta_convex",
        value: ParamValue::Float(0.2),
        unit: "",
        refs: &["lai2009"],
        note: "Peso degli spigoli convessi",
        to_confirm: false,
    },
    Param {
        key: "rw_sigma",
        value: ParamValue::Float(1.0),
        unit: "",
        refs: &["lai2009"],
        note: "Sigma della funzione esponenziale",
        to_confirm: false,
    },
    Param {
        key: "rw_faces",
        value: ParamValue::Int(30000),
        unit: "faces",
        refs: &["lai2009"],
        note: "Facce della mesh semplificata per la segmentazione",
        to_confirm: false,
    },
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn the_tables_match_the_plugin() {
        // 111 references and 78 parameters in `evidence.py`
        assert_eq!(REFERENCES.len(), 111);
        assert_eq!(PARAMS.len(), 78);
        let keys: HashSet<_> = REFERENCES.iter().map(|r| r.key).collect();
        assert_eq!(keys.len(), REFERENCES.len(), "duplicate reference keys");
        let keys: HashSet<_> = PARAMS.iter().map(|p| p.key).collect();
        assert_eq!(keys.len(), PARAMS.len(), "duplicate parameter keys");
    }

    #[test]
    fn default_is_not_to_be_confirmed() {
        let p = Param {
            key: "x",
            value: ParamValue::Int(1),
            unit: "mm",
            refs: &[],
            note: "x",
            to_confirm: false,
        };
        assert!(!p.to_confirm);
    }

    #[test]
    fn tooltip_with_to_confirm_contains_the_phrase() {
        let p = Param {
            key: "x",
            value: ParamValue::Int(1),
            unit: "mm",
            refs: &[],
            note: "x",
            to_confirm: true,
        };
        assert!(p.tooltip().contains(TO_CONFIRM_SUFFIX));
        assert!(p.tooltip().starts_with('x'));
        assert_eq!(TO_CONFIRM_SUFFIX, " (da confermare da un ortodontista)");
    }

    #[test]
    fn tooltip_without_to_confirm_has_no_phrase() {
        let p = Param {
            key: "x",
            value: ParamValue::Int(1),
            unit: "mm",
            refs: &[],
            note: "x",
            to_confirm: false,
        };
        assert!(!p.tooltip().contains(TO_CONFIRM_SUFFIX));
        assert_eq!(p.tooltip(), "x");
    }

    #[test]
    fn every_reference_of_a_parameter_exists() {
        let missing: Vec<_> = PARAMS
            .iter()
            .flat_map(|p| p.refs.iter().map(move |r| (p.key, *r)))
            .filter(|(_, r)| reference(r).is_none())
            .collect();
        assert!(missing.is_empty(), "{missing:?}");
    }

    #[test]
    fn parameters_without_a_source_are_to_be_confirmed() {
        // the plugin marks the design values with no direct source (ANO-007, ANO-101)
        for p in PARAMS {
            if p.refs.is_empty() {
                assert!(p.to_confirm, "{} has neither source nor to_confirm", p.key);
            }
        }
    }

    #[test]
    fn short_citations() {
        assert_eq!(
            reference("castroflorio2023").unwrap().short(),
            "Castroflorio 2023"
        );
        assert_eq!(
            reference("martinezlozano2024").unwrap().short(),
            "Martinez-Lozano 2024"
        );
        assert_eq!(reference("mali2021").unwrap().short(), "Ma 2021");
        assert_eq!(reference("lai2009").unwrap().short(), "Lai 2009");
        assert_eq!(
            cite(&["simon2014", "castroflorio2023"]),
            "Simon 2014; Castroflorio 2023"
        );
        assert_eq!(cite(&["nope"]), "");
        assert!(!is_upper("Lai") && is_upper("YK") && !is_upper("12"));
    }

    #[test]
    fn full_citation() {
        let s = reference("simon2014").unwrap().full();
        assert!(s.starts_with("Simon M, Keilig L"));
        assert!(s.contains("(2014). Treatment outcome"));
        assert!(s.ends_with("doi:10.1186/1472-6831-14-68 PMID:24923279"));
        // no PMID: only the DOI
        let s = reference("besl1992").unwrap().full();
        assert!(s.ends_with("doi:10.1109/34.121791"));
        // neither
        let r = Ref {
            key: "k",
            authors: "A B",
            year: 2000,
            title: "T",
            source: "S",
            doi: "",
            pmid: "",
            finding: "",
        };
        assert_eq!(r.full(), "A B (2000). T. S.");
    }

    #[test]
    fn values_of_the_defaults() {
        assert_eq!(param_f64("step_linear"), Some(0.25));
        assert_eq!(param_f64("step_rotation_round"), Some(1.5));
        assert_eq!(param_f64("max_crown_step"), Some(0.33));
        assert_eq!(param("interval_days").unwrap().as_i64(), Some(14));
        assert_eq!(param("wear_hours").unwrap().as_i64(), Some(22));
        assert_eq!(param("rw_faces").unwrap().as_i64(), Some(30000));
        assert_eq!(param("step_linear").unwrap().as_i64(), None);
        assert_eq!(param("shell_thickness").unwrap().as_f64(), Some(1.0));
        assert_eq!(param("perio_step").unwrap().entry("STAGE2"), Some(0.15));
        assert_eq!(param("perio_step").unwrap().as_f64(), None);
        assert_eq!(
            param("overcorrection").unwrap().entry("rotation"),
            Some(0.284)
        );
        assert_eq!(param("overcorrection").unwrap().entry("nope"), None);
        assert!(param("cons_line_sigma").unwrap().to_confirm);
        assert!(!param("step_linear").unwrap().to_confirm);
        assert_eq!(param("step_linear").unwrap().unit, "mm");
        assert!(param("nope").is_none());
    }

    #[test]
    fn tooltip_of_a_sourced_value_cites_it() {
        let t = param("seq_overlap").unwrap().tooltip();
        assert!(t.ends_with(" [Martinez-Lozano 2024; Ravera 2016]"), "{t}");
    }

    #[test]
    fn references_text_is_sorted_and_complete() {
        let text = references_text();
        let entries = text.lines().filter(|l| !l.starts_with("    -> ")).count();
        assert_eq!(entries, REFERENCES.len());
        let first = text.lines().next().unwrap();
        // the first author in case-insensitive order
        let mut authors: Vec<String> = REFERENCES
            .iter()
            .map(|r| r.authors.to_lowercase())
            .collect();
        authors.sort();
        assert!(first.to_lowercase().starts_with(&authors[0]));
        assert!(text.contains("\n    -> "));
    }
}
