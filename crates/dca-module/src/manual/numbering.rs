//! Tooth numbers by hand: renumber a tooth (or swap two) and mark a tooth
//! as missing or present again.
//!
//! `aligners.preparation.numbering.renumber`: tooth `src` becomes `dst`; when
//! `dst` already exists the two numbers are swapped. The seeds, the labels
//! and the frames stay consistent: the seeds exchange their numbers, the
//! labels of the arch are renamed, and the renumbered teeth are measured and
//! closed again (their number decides their axes and centre of resistance);
//! what the setup and the staging had computed for them is dropped.
//!
//! `aligners.preparation.numbering.mark-missing`: the tooth is absent
//! (agenesis, extraction). Its seed, labels, frame, solid and movements are
//! removed and its number is kept in `AlignersState::missing`, which the
//! seeds and the brush respect and the later stages read as "no tooth
//! here"; its space stays empty. With `missing: false` the tooth is present
//! again (to be seeded or painted, then rebuilt).

use dc_case::{CaseState, Fdi};
use dc_command::{CommandError, JobCtx};
use dc_module_api::CommandOutput;
use dca_algo::anatomy::Arch;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::json;

use super::common::{
    Forgotten, arch_labels, arch_name, data_of, fdi_of, finish, forget_tooth, invalid, run_command,
    store_labels,
};

/// Parameters of the renumbering.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct RenumberParams {
    /// Current number (FDI) of the tooth (required).
    pub src: u8,
    /// New number (FDI) of the tooth (required); a tooth that already has it
    /// takes the number `src`.
    pub dst: u8,
}

/// Parameters of mark-missing.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct MissingParams {
    /// Tooth number (FDI) (required).
    pub fdi: u8,
    /// `true` (default): the tooth is missing; `false`: it is present again.
    pub missing: bool,
}

impl Default for MissingParams {
    fn default() -> Self {
        Self {
            fdi: 0,
            missing: true,
        }
    }
}

/// `true` when the labels of an arch use `f`.
fn has_label(labels: &[u16], f: Fdi) -> bool {
    labels.contains(&u16::from(f.get()))
}

pub(crate) fn run_renumber(
    state: &CaseState,
    p: RenumberParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let (src, dst) = (fdi_of(p.src)?, fdi_of(p.dst)?);
    if src == dst {
        return Err(invalid("the two numbers are the same"));
    }
    if Arch::of(src) != Arch::of(dst) {
        return Err(invalid(format!(
            "teeth {src} and {dst} are in different arches"
        )));
    }
    let arch = Arch::of(src);
    let mut data = data_of(state)?;
    if data.missing.contains(&src) {
        return Err(invalid(format!(
            "tooth {src} is marked missing: there is nothing to renumber"
        )));
    }
    // the labels of the arch, when it has any
    let labels = if data.labels.contains_key(&arch.role()) {
        Some(arch_labels(state, &data, arch)?)
    } else {
        None
    };
    let seed_of = |data: &dca_model::AlignersState, f: Fdi| data.seeds.iter().any(|s| s.fdi == f);
    let has_src = labels.as_ref().is_some_and(|l| has_label(&l.labels, src))
        || seed_of(&data, src)
        || data.tooth_frames.contains_key(&src);
    if !has_src {
        return Err(invalid(format!(
            "tooth {src} does not exist (no seed, labels or frame)"
        )));
    }
    let swap = labels.as_ref().is_some_and(|l| has_label(&l.labels, dst))
        || seed_of(&data, dst)
        || data.tooth_frames.contains_key(&dst);

    let mut next = state.clone();
    let mut stale = Vec::new();
    // labels
    let mut relabelled = 0;
    if let Some(l) = &labels {
        let (s, d) = (u16::from(src.get()), u16::from(dst.get()));
        let new: Vec<u16> = l
            .labels
            .iter()
            .map(|&x| {
                if x == s {
                    d
                } else if swap && x == d {
                    s
                } else {
                    x
                }
            })
            .collect();
        relabelled = new.iter().zip(&l.labels).filter(|(a, b)| a != b).count();
        store_labels(&mut next, &mut data, &mut stale, arch, &l.scan, new);
    }
    // seeds: the two exchange their numbers
    for seed in &mut data.seeds {
        if seed.fdi == src {
            seed.fdi = dst;
        } else if swap && seed.fdi == dst {
            seed.fdi = src;
        }
    }
    if !swap {
        data.missing.remove(&dst); // the number is taken: the tooth is there
    }
    // what was computed for the two numbers is wrong now
    let had_frames = data.tooth_frames.contains_key(&src) || data.tooth_frames.contains_key(&dst);
    let mut to_rebuild = Vec::new();
    if data.crowns.contains_key(&src) {
        to_rebuild.push(dst);
    }
    if swap && data.crowns.contains_key(&dst) {
        to_rebuild.push(src);
    }
    let mut forgotten = Forgotten::default();
    for f in [src, dst] {
        forgotten = forgotten.merge(forget_tooth(&mut data, &mut stale, f));
    }
    finish(&mut next, data, &stale);

    // the renumbered teeth are measured and closed again
    let mut rebuilt = Vec::new();
    if had_frames {
        let (s, _) = run_command(
            "aligners.teeth.frames",
            &next,
            json!({ "arch": arch_name(arch) }),
            ctx,
        )?;
        next = s;
        for f in to_rebuild {
            let (s, _) = run_command(
                "aligners.preparation.reconstruction.rebuild-tooth",
                &next,
                json!({ "fdi": f.get() }),
                ctx,
            )?;
            next = s;
            rebuilt.push(f.get());
        }
        rebuilt.sort_unstable();
    }
    Ok((
        next,
        json!({
            "src": src.get(),
            "dst": dst.get(),
            "swap": swap,
            "relabelled_vertices": relabelled,
            "rebuilt": rebuilt,
            "dropped": forgotten.json(),
            "setup_outdated": forgotten.movement,
        }),
    ))
}

pub(crate) fn run_missing(
    state: &CaseState,
    p: MissingParams,
    ctx: &JobCtx,
) -> Result<CommandOutput, CommandError> {
    ctx.check_cancelled()?;
    let f = fdi_of(p.fdi)?;
    let arch = Arch::of(f);
    let mut data = data_of(state)?;
    let mut next = state.clone();
    if !p.missing {
        let was = data.missing.remove(&f);
        finish(&mut next, data, &[]);
        return Ok((
            next,
            json!({ "fdi": f.get(), "missing": false, "was_missing": was }),
        ));
    }
    let mut stale = Vec::new();
    data.missing.insert(f);
    let seeds = data.seeds.len();
    data.seeds.retain(|s| s.fdi != f);
    let seed_removed = data.seeds.len() != seeds;
    // labels to gingiva
    let mut relabelled = 0;
    if data.labels.contains_key(&arch.role()) && state.case.scan(arch.role()).is_some() {
        let l = arch_labels(state, &data, arch)?;
        let want = u16::from(f.get());
        if has_label(&l.labels, f) {
            let new: Vec<u16> = l
                .labels
                .iter()
                .map(|&x| if x == want { 0 } else { x })
                .collect();
            relabelled = l.labels.iter().filter(|&&x| x == want).count();
            store_labels(&mut next, &mut data, &mut stale, arch, &l.scan, new);
        }
    }
    let forgotten = forget_tooth(&mut data, &mut stale, f);
    finish(&mut next, data, &stale);
    Ok((
        next,
        json!({
            "fdi": f.get(),
            "missing": true,
            "labels": relabelled,
            "seed": seed_removed,
            "dropped": forgotten.json(),
            "setup_outdated": forgotten.movement,
        }),
    ))
}
