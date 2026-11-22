//! Tooth boundaries along the arch by dynamic programming (`autoseed._dp`).

use super::numpy::interp;
use super::{
    DIASTEMA_COST, DIASTEMA_MM, END_ZONE, GAP_COST, MISSING_COST, MISSING_GAP, NONCROWN_COST,
    VALLEY_COST, VALLEY_MM,
};

/// "Infinite" cost of the plugin (a finite number: sums stay comparable).
const INF: f64 = 1e18;
/// Relative tolerance on the bucco-lingual width of a tooth.
const SIGMA_BL: f64 = 0.2;
/// Weight of the bucco-lingual width term.
const LAM_BL: f64 = 0.7;

/// A tooth-sequence problem on the bins `[0, nb)` of the arch profile
/// (`nb = dip.len()`).
#[derive(Debug, Clone, Copy)]
pub struct DpProblem<'a> {
    /// Expected mesio-distal width of every tooth, in bins.
    pub w_bins: &'a [f64],
    /// Expected bucco-lingual width of every tooth (relative).
    pub bl_exp: &'a [f64],
    /// Interproximal valley depth of every bin (mm).
    pub dip: &'a [f64],
    /// Cumulative relative bucco-lingual width, `nb + 1` entries from 0.
    pub bl_cum: &'a [f64],
    /// Relative tolerance on the mesio-distal width of every tooth.
    pub sigma: &'a [f64],
    /// Optional anchor of the end of a tooth: (tooth, bin, sigma in bins).
    pub mid: Option<(usize, f64, f64)>,
    /// Crown score 0-1 of every bin.
    pub crown: &'a [f64],
    /// Teeth that may be missing.
    pub skippable: &'a [bool],
    /// Bin size (mm).
    pub bin_mm: f64,
    /// Free ends of the arch ([`FreeEnds`]); `None`: the plugin's ends,
    /// the sequence covers the profile up to `END_ZONE + 3` mm of each end.
    pub free_ends: Option<FreeEnds<'a>>,
}

/// The ends of the arch left to the data ([`DpProblem::free_ends`]): the
/// first tooth may start, and the last one end, farther than the plugin's
/// slack from the ends of the profile when what is left out stays lower
/// than the crown top of that end tooth by [`FreeEnds::drop_mm`] (the
/// tuberosity, the retromolar pad or the back wall of a model behind the
/// last molar, not a crown). What is left beyond the slack costs
/// [`FreeEnds::tail_cost`] per mm of crown-like surface. Without it the
/// sequence must reach both ends of the profile, and a profile running past
/// the last molars gets a tooth too many there or stretched teeth.
#[derive(Debug, Clone, Copy)]
pub struct FreeEnds<'a> {
    /// Crown top height of every bin (mm).
    pub h: &'a [f64],
    /// Cost per mm of crown-like surface left out beyond the slack.
    pub tail_cost: f64,
    /// What is left out stays this much (mm) below the end tooth's top.
    pub drop_mm: f64,
    /// Bins next to the end tooth not compared (its own distal slope).
    pub guard: usize,
}

/// Every tooth `j` is either present, a segment `[a, b)` whose cost is the
/// deviation from the expected mesio-distal width `w_bins[j]` and
/// bucco-lingual width `bl_exp[j]` (relative), plus [`NONCROWN_COST`] per mm
/// of gingiva-like surface in it, plus the valley cost of its end boundary
/// ([`VALLEY_COST`]: a boundary must lie in an interproximal valley, the
/// two ends of the arch too), optionally preceded by an interdental space
/// (diastema); or missing (only if `skippable[j]`), leaving a space of 0 to
/// [`MISSING_GAP`] of its width (closed or partly closed) that must look
/// like gingiva, at the prior cost [`MISSING_COST`]. A space is gingiva when
/// the crown score is low or it is a deep valley between the neighbours.
///
/// Returns the total cost and, per tooth, its segment or `None` when it is
/// missing.
pub fn dp(p: &DpProblem<'_>) -> (f64, Vec<Option<(usize, usize)>>) {
    let n = p.w_bins.len();
    let nb = p.dip.len();
    let bin = p.bin_mm;
    let slack = ((END_ZONE + 3.0) / bin) as usize;
    let bcost: Vec<f64> = p
        .dip
        .iter()
        .map(|&d| interp(d, &VALLEY_MM, &VALLEY_COST))
        .collect();
    let gum: Vec<f64> = (0..nb)
        .map(|i| (1.0 - p.crown[i]).max(((p.dip[i] - 0.5) / 1.0).clamp(0.0, 1.0)))
        .collect();
    let cumsum = |f: &dyn Fn(usize) -> f64| -> Vec<f64> {
        let mut acc = 0.0;
        let mut out = Vec::with_capacity(nb + 1);
        out.push(0.0);
        for i in 0..nb {
            acc += f(i);
            out.push(acc * bin);
        }
        out
    };
    // gingiva-like mm in [0, a) and crown-like mm in [0, a)
    let ncr = cumsum(&|i| 1.0 - p.crown[i]);
    let ngu = cumsum(&|i| 1.0 - gum[i]);
    let mut cost = vec![INF; nb];
    let head = (slack + 1).min(nb);
    cost[..head].copy_from_slice(&bcost[..head]);
    let dmax = (DIASTEMA_MM / bin) as usize;
    // free ends: crown-like mm in [0, a), crown top over [0, i] and [i, nb)
    let free = p.free_ends.map(|fe| {
        let ccr = cumsum(&|i| p.crown[i]);
        let mut pre = Vec::with_capacity(nb);
        let mut m = f64::NEG_INFINITY;
        for &x in &fe.h[..nb] {
            m = m.max(x);
            pre.push(m);
        }
        let mut suf = vec![f64::NEG_INFINITY; nb + 1];
        for i in (0..nb).rev() {
            suf[i] = suf[i + 1].max(fe.h[i]);
        }
        (fe, ccr, pre, suf)
    });

    // best start of a space [a - g, a), g = 0..=gmax, ending at every bin a
    let space = |prev: &[f64], gmax: usize, fixed: f64| -> (Vec<f64>, Vec<usize>) {
        let mut c_out = vec![0.0; nb];
        let mut from = vec![0usize; nb];
        for a in 0..nb {
            let mut best = (f64::NAN, 0usize);
            for g in 0..=gmax {
                let (valid, ac) = if a >= g { (true, a - g) } else { (false, 0) };
                let c = (if valid { prev[ac] } else { INF })
                    + GAP_COST * (ngu[a] - ngu[ac])
                    + if g > 0 { fixed } else { 0.0 };
                if g == 0 || c < best.0 {
                    best = (c, ac);
                }
            }
            c_out[a] = best.0;
            from[a] = best.1;
        }
        (c_out, from)
    };

    // per tooth: where the previous tooth ended, and the start of this one (None: missing)
    let mut backs: Vec<(Vec<usize>, Vec<Option<usize>>)> = Vec::with_capacity(n);
    for j in 0..n {
        let (w, blx) = (p.w_bins[j], p.bl_exp[j]);
        let (start_cost, start_from) = if j > 0 && dmax > 0 {
            space(&cost, dmax, DIASTEMA_COST)
        } else {
            (cost.clone(), (0..nb).collect())
        };
        let l_lo = ((0.6 * w) as i64).max(1);
        let l_hi = (1.5 * w) as i64 + 2;
        let mut pres = vec![INF; nb];
        let mut t_start = vec![0usize; nb];
        // the first tooth with free ends: its crown top, the segment grown with its length
        let first_free = if j == 0 { free.as_ref() } else { None };
        for a in 0..nb {
            let mut best = (f64::NAN, 0usize);
            let mut top = first_free.map_or(f64::NEG_INFINITY, |(fe, ..)| {
                let lo = (a as i64 - l_lo + 1).max(0) as usize;
                fe.h[lo..a]
                    .iter()
                    .copied()
                    .fold(f64::NEG_INFINITY, f64::max)
            });
            for l in l_lo..l_hi {
                let start = a as i64 - l;
                let (valid, ac) = if start >= 0 {
                    (true, start as usize)
                } else {
                    (false, 0)
                };
                let prev = match first_free {
                    Some((fe, ccr, pre, _)) if valid && ac > slack => {
                        top = top.max(fe.h[ac]);
                        // what is left before the tooth: lower than its top
                        let looked = ac.saturating_sub(fe.guard);
                        if looked == 0 || pre[looked - 1] < top - fe.drop_mm {
                            bcost[ac] + fe.tail_cost * (ccr[ac] - ccr[slack + 1])
                        } else {
                            INF
                        }
                    }
                    Some((fe, ..)) if valid => {
                        top = top.max(fe.h[ac]);
                        start_cost[ac]
                    }
                    _ if valid => start_cost[ac],
                    _ => INF,
                };
                let lf = l as f64;
                let mean_bl = (p.bl_cum[a] - p.bl_cum[ac]) / lf;
                let dw = (lf - w) / (p.sigma[j] * w);
                let dbl = (mean_bl - blx) / (SIGMA_BL * blx);
                let c = prev + dw * dw + LAM_BL * (dbl * dbl) + NONCROWN_COST * (ncr[a] - ncr[ac]);
                if l == l_lo || c < best.0 {
                    best = (c, ac);
                }
            }
            pres[a] = best.0;
            t_start[a] = best.1;
        }
        for (a, v) in pres.iter_mut().enumerate() {
            *v = if *v < INF / 2.0 {
                *v + if j < n - 1 { bcost[a] } else { 0.0 }
            } else {
                INF
            };
            if let Some((tooth, mid_bin, mid_sigma)) = p.mid
                && tooth == j
            {
                *v = if *v < INF / 2.0 {
                    let d = (a as f64 - mid_bin) / mid_sigma;
                    *v + d * d
                } else {
                    INF
                };
            }
        }
        let t_from: Vec<usize> = t_start.iter().map(|&s| start_from[s]).collect();
        if p.skippable[j] {
            let (absn, a_from) = space(&cost, (MISSING_GAP * w) as usize, 0.0);
            let mut from = Vec::with_capacity(nb);
            let mut st = Vec::with_capacity(nb);
            for a in 0..nb {
                let ab = absn[a] + MISSING_COST;
                if ab < pres[a] {
                    cost[a] = ab;
                    from.push(a_from[a]);
                    st.push(None);
                } else {
                    cost[a] = pres[a];
                    from.push(t_from[a]);
                    st.push(Some(t_start[a]));
                }
            }
            backs.push((from, st));
        } else {
            cost = pres;
            backs.push((t_from, t_start.into_iter().map(Some).collect()));
        }
    }
    let end_lo = nb.saturating_sub(1 + slack);
    // free ends: the last tooth may end before end_lo when what is left after
    // it stays lower than its top
    let last_start = backs.last().map(|(_, st)| st);
    let first = if free.is_some() { 0 } else { end_lo };
    let mut b = first;
    let mut total = f64::NAN;
    for i in first..nb {
        let f = match (&free, last_start) {
            (Some((fe, ccr, _, suf)), Some(st)) if i < end_lo => match st[i] {
                Some(s) if s < i && cost[i] < INF / 2.0 => {
                    let top = fe.h[s..i].iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    if suf[(i + fe.guard).min(nb)] < top - fe.drop_mm {
                        cost[i] + bcost[i] + fe.tail_cost * (ccr[end_lo] - ccr[i])
                    } else {
                        INF
                    }
                }
                _ => INF,
            },
            _ => cost[i] + bcost[i],
        };
        if i == first || f < total {
            total = f;
            b = i;
        }
    }
    let mut segs = vec![None; n];
    for j in (0..n).rev() {
        let (from, st) = (&backs[j].0[b], backs[j].1[b]);
        if let Some(s) = st {
            segs[j] = Some((s, b));
        }
        b = *from;
    }
    (total, segs)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Three teeth of 8 bins in valleys every 8 bins: the boundaries land
    /// on the valleys.
    #[test]
    fn boundaries_follow_the_valleys() {
        let nb = 30;
        let mut dip = vec![0.0; nb];
        for k in [3, 11, 19, 27] {
            dip[k] = 1.6;
        }
        let bl_cum: Vec<f64> = (0..=nb).map(|i| i as f64).collect();
        let crown = vec![1.0; nb];
        let p = DpProblem {
            w_bins: &[8.0, 8.0, 8.0],
            bl_exp: &[1.0, 1.0, 1.0],
            dip: &dip,
            bl_cum: &bl_cum,
            sigma: &[0.1, 0.1, 0.1],
            mid: None,
            crown: &crown,
            skippable: &[false, true, false],
            bin_mm: 0.25,
            free_ends: None,
        };
        let (cost, segs) = dp(&p);
        assert_eq!(segs, vec![Some((3, 11)), Some((11, 19)), Some((19, 27))]);
        assert!(cost < 0.0, "{cost}");
    }

    /// Valley depth of a crown-top profile as `autoseed::arch_profiles`
    /// measures it: highest point within 2 mm (8 bins) minus the height.
    fn dips(h: &[f64]) -> Vec<f64> {
        let n = h.len() as i64;
        (0..n)
            .map(|i| {
                let m = (i - 8..=i + 8)
                    .map(|k| h[k.clamp(0, n - 1) as usize])
                    .fold(f64::NEG_INFINITY, f64::max);
                m - h[i as usize]
            })
            .collect()
    }

    /// Three molars of 10 mm (40 bins, valleys between them), then `tail`:
    /// what lies behind the last one.
    fn molars_then(tail: &[f64]) -> Vec<f64> {
        let mut h: Vec<f64> = (0..120)
            .map(|i| -1.6 + 1.6 * (std::f64::consts::PI * (i % 40) as f64 / 40.0).sin())
            .collect();
        h.extend_from_slice(tail);
        h
    }

    fn three_molars(h: &[f64], free: bool) -> (f64, Vec<Option<(usize, usize)>>) {
        let nb = h.len();
        let dip = dips(h);
        let bl_cum: Vec<f64> = (0..=nb).map(|i| i as f64).collect();
        // a noisy scan: the gum behind the molars looks like enamel
        let crown = vec![1.0; nb];
        dp(&DpProblem {
            w_bins: &[40.0, 40.0, 40.0],
            bl_exp: &[1.0, 1.0, 1.0],
            dip: &dip,
            bl_cum: &bl_cum,
            sigma: &[0.1, 0.1, 0.1],
            mid: None,
            crown: &crown,
            skippable: &[false, false, false],
            bin_mm: 0.25,
            free_ends: free.then_some(FreeEnds {
                h,
                tail_cost: 2.0,
                drop_mm: 0.5,
                guard: 4,
            }),
        })
    }

    /// 10 mm of gum behind the last molar (the tuberosity or the back wall
    /// of a model: up to 1 mm below the crown tops, then down): the plugin's
    /// ends make the teeth reach it, the free ends leave it out and the
    /// teeth stay on their valleys.
    #[test]
    fn the_gum_behind_the_last_molar_is_left_out() {
        let tail: Vec<f64> = (0..40)
            .map(|i| {
                if i < 8 {
                    -1.6 + 0.6 * i as f64 / 8.0
                } else {
                    -1.0 - 4.0 * (i - 8) as f64 / 32.0
                }
            })
            .collect();
        let h = molars_then(&tail);
        let teeth = vec![Some((0, 40)), Some((40, 80)), Some((80, 120))];
        let (_, forced) = three_molars(&h, false);
        assert_ne!(forced, teeth, "the plugin's ends reach the gum");
        assert!(forced[2].is_some_and(|(_, b)| b >= h.len() - 21));
        let (_, free) = three_molars(&h, true);
        assert_eq!(free, teeth);
    }

    /// What lies behind the last molar as high as a crown is not left out:
    /// the free ends give the plugin's teeth.
    #[test]
    fn a_crown_high_region_is_not_left_out() {
        let tail: Vec<f64> = (0..40)
            .map(|i| -1.6 + 1.8 * (std::f64::consts::PI * i as f64 / 40.0).sin())
            .collect();
        let h = molars_then(&tail);
        assert_eq!(three_molars(&h, true), three_molars(&h, false));
    }
}
