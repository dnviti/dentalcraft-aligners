//! The plugin unit cases (`tests/unit/test_setup_conservativo.py`), on the
//! same synthetic lower arch: teeth in contact on a parabola, with single
//! teeth shifted, rotated or resized.

use std::collections::BTreeMap;

use dc_case::Fdi;
use glam::{DMat3, DMat4, DVec2, DVec3};

use super::line::{arc, interior, interp, line_normals, project, tangent_at};
use super::*;

const SEQ_N: [u8; 14] = [47, 46, 45, 44, 43, 42, 41, 31, 32, 33, 34, 35, 36, 37];

fn fdi(n: u8) -> Fdi {
    Fdi::new(n).unwrap()
}

fn seq() -> Vec<Fdi> {
    SEQ_N.iter().map(|&n| fdi(n)).collect()
}

fn width_of(i: u8) -> f64 {
    match i {
        7 => 10.5,
        6 => 11.0,
        5 | 4 => 7.0,
        3 => 6.8,
        2 => 5.9,
        _ => 5.4,
    }
}

/// The parabola y = 22 - 0.045 x^2 and its arc length (0 at x = 0).
fn curve() -> (Vec<DVec2>, Vec<f64>) {
    let n = 20001;
    // numpy.linspace: i * step + start, the last value exactly the stop
    let step = 70.0 / (n - 1) as f64;
    let p: Vec<DVec2> = (0..n)
        .map(|i| {
            let x = if i == n - 1 {
                35.0
            } else {
                i as f64 * step - 35.0
            };
            DVec2::new(x, 22.0 - 0.045 * (x * x))
        })
        .collect();
    let mut s = vec![0.0];
    for w in p.windows(2) {
        s.push(s[s.len() - 1] + (w[1] - w[0]).length());
    }
    let xs: Vec<f64> = p.iter().map(|q| q.x).collect();
    let s0 = interp(0.0, &xs, &s);
    s.iter_mut().for_each(|v| *v -= s0);
    (p, s)
}

fn rotz(v: DVec3, a: f64) -> DVec3 {
    let (s, c) = a.sin_cos();
    DVec3::new(c * v.x - s * v.y, s * v.x + c * v.y, v.z)
}

struct Synth {
    teeth: BTreeMap<Fdi, Tooth>,
    cusps: BTreeMap<Fdi, Vec<DVec3>>,
    out_dir: BTreeMap<Fdi, DVec3>,
}

#[derive(Default)]
struct Mal {
    shift: Vec<(u8, DVec3)>,
    rot: Vec<(u8, f64)>,
    widths: Vec<(u8, f64)>,
    /// Space (mm) along the arch on the mesial side of a tooth.
    gaps: Vec<(u8, f64)>,
}

/// `make_arch`: a regular synthetic lower arch with shifted, rotated or
/// resized teeth, and spaces.
fn make_arch(m: &Mal) -> Synth {
    let (p, s) = curve();
    let xs: Vec<f64> = p.iter().map(|q| q.x).collect();
    let ys: Vec<f64> = p.iter().map(|q| q.y).collect();
    let mut a = Synth {
        teeth: BTreeMap::new(),
        cusps: BTreeMap::new(),
        out_dir: BTreeMap::new(),
    };
    let get = |v: &[(u8, f64)], f: u8| v.iter().find(|x| x.0 == f).map(|x| x.1);
    for (quadrant, sign) in [(4u8, -1.0), (3u8, 1.0)] {
        let mut acc = 0.0;
        for i in 1..8u8 {
            let f = quadrant * 10 + i;
            let w = get(&m.widths, f).unwrap_or(width_of(i));
            acc += get(&m.gaps, f).unwrap_or(0.0);
            let sc = sign * (acc + w / 2.0);
            acc += w;
            let (x, y) = (interp(sc, &s, &xs), interp(sc, &s, &ys));
            let c = DVec3::new(x, y, 0.0);
            let tg = DVec3::new(1.0, -0.09 * x, 0.0).normalize();
            let mut md = if sign < 0.0 { tg } else { -tg };
            let mut out = DVec3::new(-tg.y, tg.x, 0.0);
            if out.y < 0.0 {
                out = -out;
            }
            if out.dot(DVec3::new(x, y, 0.0)) < 0.0 {
                out = -out;
            }
            let mut cs = match i {
                1 | 2 => vec![c + tg * w * 0.3, c - tg * w * 0.3],
                6 | 7 => vec![c + tg * w * 0.25, c - tg * w * 0.25],
                _ => vec![c],
            };
            let (mut cm, mut cd) = (c + md * w / 2.0, c - md * w / 2.0);
            let ang = get(&m.rot, f).unwrap_or(0.0).to_radians();
            if ang != 0.0 {
                cs = cs.iter().map(|&q| c + rotz(q - c, ang)).collect();
                cm = c + rotz(cm - c, ang);
                cd = c + rotz(cd - c, ang);
                md = rotz(md, ang);
            }
            let d = m
                .shift
                .iter()
                .find(|x| x.0 == f)
                .map_or(DVec3::ZERO, |x| x.1);
            a.teeth.insert(
                fdi(f),
                Tooth {
                    width_md: w,
                    contact_m: cm + d,
                    contact_d: cd + d,
                    e_md: md,
                    md_sign: 1.0,
                    e_ax: DVec3::Z,
                    edge_md: false,
                },
            );
            a.cusps.insert(fdi(f), cs.iter().map(|&q| q + d).collect());
            a.out_dir.insert(fdi(f), out);
        }
    }
    a
}

fn regular() -> Synth {
    make_arch(&Mal::default())
}

/// `_crowded`: the regular arch with the widths (and contacts) of `widths`.
fn crowded(widths: &[(u8, f64)]) -> Synth {
    let mut a = regular();
    let w = make_arch(&Mal {
        widths: widths.to_vec(),
        ..Mal::default()
    });
    for (f, t) in a.teeth.iter_mut() {
        let c = mean(&a.cusps[f]);
        let ww = w.teeth[f].width_md;
        t.width_md = ww;
        t.contact_m = c + t.e_md * ww / 2.0;
        t.contact_d = c - t.e_md * ww / 2.0;
    }
    a
}

fn setup(a: &Synth, cfg: &Config) -> SetupResult {
    conservative_setup(
        &a.teeth,
        &seq(),
        &a.cusps,
        cfg,
        None,
        &[],
        None,
        &BTreeMap::new(),
    )
}

fn after(res: &SetupResult, cusps: &BTreeMap<Fdi, Vec<DVec3>>) -> BTreeMap<Fdi, Vec<DVec3>> {
    cusps
        .iter()
        .map(|(f, c)| {
            let moved = match res.targets.get(f) {
                Some(d) => c.iter().map(|&p| d.transform_point3(p)).collect(),
                None => c.clone(),
            };
            (*f, moved)
        })
        .collect()
}

fn moved(res: &SetupResult, cusps: &BTreeMap<Fdi, Vec<DVec3>>) -> BTreeMap<Fdi, f64> {
    let a = after(res, cusps);
    cusps
        .iter()
        .map(|(f, c)| {
            let d = c
                .iter()
                .zip(&a[f])
                .map(|(p, q)| (*q - *p).length())
                .fold(0.0, f64::max);
            (*f, d)
        })
        .collect()
}

fn max_dist(l: &[DVec3], p: &[DVec3]) -> f64 {
    line_distance(l, p).into_iter().fold(0.0, f64::max)
}

fn angle(d: &DMat4) -> f64 {
    let r = DMat3::from_mat4(*d);
    let tr = r.x_axis.x + r.y_axis.y + r.z_axis.z;
    ((tr - 1.0) / 2.0).clamp(-1.0, 1.0).acos().to_degrees()
}

fn mean_z(p: &[DVec3]) -> f64 {
    p.iter().map(|q| q.z).sum::<f64>() / p.len() as f64
}

/// `full`: every tooth aligned and every step levelled (the posterior teeth
/// with the tolerances of the anterior ones).
fn full(edit: impl FnOnce(&mut Config)) -> Config {
    let mut c = Config {
        align_scope: AlignScope::All,
        level_tol: 0.0,
        ..Config::default()
    };
    edit(&mut c);
    c.post_normal_tol = c.normal_tol;
    c.post_rot_tol_deg = c.rot_tol_deg;
    c
}

fn has(res: &SetupResult, pred: impl Fn(&Note) -> bool) -> bool {
    res.log.iter().any(pred)
}

// ---- the line of the cusps ------------------------------------------------------------------------

#[test]
fn the_line_passes_through_the_cusps_of_a_regular_arch() {
    let a = regular();
    let l = cusp_line(&a.cusps, &seq(), 5.0, &[], false);
    for f in seq() {
        assert!(max_dist(&l, &a.cusps[&f]) < 0.12, "{f}");
    }
}

#[test]
fn the_line_is_not_dragged_by_a_tooth_out_of_the_arch() {
    let a = make_arch(&Mal {
        shift: vec![(32, DVec3::new(0.0, -1.5, 0.0))],
        ..Mal::default()
    });
    let l = cusp_line(&a.cusps, &seq(), 5.0, &[], false);
    let d = line_distance(&l, &a.cusps[&fdi(32)]);
    assert!(d.iter().sum::<f64>() / d.len() as f64 > 0.7);
}

#[test]
fn the_line_runs_from_the_first_to_the_last_tooth() {
    let a = regular();
    let l = cusp_line(&a.cusps, &seq(), 5.0, &[], false);
    let near = |f: u8, p: DVec3| {
        a.cusps[&fdi(f)]
            .iter()
            .map(|c| (c.truncate() - p.truncate()).length())
            .fold(f64::INFINITY, f64::min)
    };
    assert!(near(47, l[0]) < 4.5);
    assert!(near(37, l[l.len() - 1]) < 4.5);
}

// ---- isotonic regression ------------------------------------------------------------------------------

#[test]
fn weighted_pava() {
    let u = isotonic(
        &[1.0, 3.0, 2.0, 4.0],
        &[1.0; 4],
        f64::NEG_INFINITY,
        f64::INFINITY,
    );
    assert_eq!(u, vec![1.0, 2.5, 2.5, 4.0]);
}

#[test]
fn pava_with_bounds() {
    assert_eq!(isotonic(&[0.0, 5.0], &[1.0; 2], 1.0, 3.0), vec![1.0, 3.0]);
}

#[test]
fn neighbouring_fixed_teeth_are_not_crowding() {
    let ch = solve_chain(
        &[0.0, 2.0, 1.5, 3.0],
        &[1.0; 4],
        &[false, true, true, false],
        false,
        false,
    );
    assert_eq!(ch.deficit, 0.0);
    assert_eq!((ch.u[1], ch.u[2]), (2.0, 1.5));
}

#[test]
fn stretches_between_fixed_teeth() {
    let fixed = [true, false, false, true];
    let ch = solve_chain(&[0.0, 1.0, 0.5, 2.0], &[1.0; 4], &fixed, false, false);
    assert_eq!(ch.deficit, 0.0);
    assert_eq!(ch.u, vec![0.0, 0.75, 0.75, 2.0]);
    let ch = solve_chain(&[2.0, 1.0, 1.0, 0.5], &[1.0; 4], &fixed, false, false);
    assert!((ch.deficit - 1.5).abs() < 1e-12);
}

#[test]
fn the_last_tooth_of_a_side_never_goes_distally() {
    // the chain needs 1 mm more than the room between its last teeth: the plugin pushes the
    // last teeth distally, the port finds crowding (the space is made by the therapy)
    let u0 = [1.0, 1.0, 0.0, 0.0];
    let free = [false; 4];
    let plugin = solve_chain(&u0, &[1.0; 4], &free, true, false);
    assert_eq!(plugin.deficit, 0.0);
    assert!(plugin.u[0] < u0[0] && plugin.u[3] > u0[3]);
    let port = solve_chain(&u0, &[1.0; 4], &free, true, true);
    assert!((port.deficit - 1.0).abs() < 1e-12);
    assert_eq!((port.segs[0].left, port.segs[0].right), (-1, 4));
}

#[test]
fn the_teeth_behind_a_space_close_it_against_a_fixed_tooth() {
    // a space of 1.5 mm before the fixed tooth 2: the plugin leaves it, the port brings the
    // free teeth mesially against the fixed tooth, none of them distally
    let u0 = [0.0, 0.5, 2.0];
    let fixed = [false, false, true];
    let plugin = solve_chain(&u0, &[1.0; 3], &fixed, true, false);
    assert_eq!(plugin.u, vec![0.25, 0.25, 2.0]);
    let port = solve_chain(&u0, &[1.0; 3], &fixed, true, true);
    assert_eq!(port.u, vec![2.0, 2.0, 2.0]);
    assert_eq!((port.deficit, port.spacing), (0.0, 0.0));
    // without a fixed tooth the chain closes where its weights say, the last teeth no further
    // out than they were
    let port = solve_chain(&[0.0, 0.0, 1.0, 1.0], &[1.0; 4], &[false; 4], true, true);
    assert_eq!(port.u, vec![0.5; 4]);
}

// ---- conservative setup -------------------------------------------------------------------------------

#[test]
fn a_regular_arch_does_not_move() {
    let res = setup(&regular(), &full(|_| {}));
    assert!(res.targets.is_empty());
    assert!(res.ipr.is_empty());
}

#[test]
fn only_the_shifted_tooth_moves() {
    let a = make_arch(&Mal {
        shift: vec![(32, DVec3::new(0.0, -1.2, 0.0))],
        ..Mal::default()
    });
    let res = setup(&a, &full(|_| {}));
    let mv = moved(&res, &a.cusps);
    assert!(mv[&fdi(32)] > 0.6);
    for f in seq() {
        if f != fdi(32) {
            assert!(mv[&f] < 0.2, "{f}");
        }
    }
    assert!(max_dist(&res.line, &after(&res, &a.cusps)[&fdi(32)]) < 0.35);
}

#[test]
fn molars_held_and_crowding_spread() {
    let widths = [
        (43, 7.3),
        (42, 6.15),
        (41, 5.65),
        (33, 6.3),
        (32, 5.65),
        (31, 5.15),
    ];
    let mut a = regular();
    let w = make_arch(&Mal {
        widths: widths.to_vec(),
        ..Mal::default()
    });
    for (f, t) in a.teeth.iter_mut() {
        let c = mean(&a.cusps[f]);
        t.width_md = w.teeth[f].width_md;
        t.contact_m = c + t.e_md * t.width_md / 2.0;
        t.contact_d = c - t.e_md * t.width_md / 2.0;
    }
    let res = setup(&a, &full(|_| {}));
    let mv = moved(&res, &a.cusps);
    for f in [47, 46, 36, 37] {
        assert_eq!(mv[&fdi(f)], 0.0, "{f}");
    }
    assert!(res.ipr.is_empty());
    assert!(mv.values().copied().fold(0.0, f64::max) < 1.6);
    assert!(res.overlap_max <= 0.16);
}

#[test]
fn teeth_out_of_the_aligner_do_not_move() {
    let a = make_arch(&Mal {
        shift: vec![
            (32, DVec3::new(0.0, -1.2, 0.0)),
            (42, DVec3::new(0.0, -1.2, 0.0)),
        ],
        ..Mal::default()
    });
    let res = conservative_setup(
        &a.teeth,
        &seq(),
        &a.cusps,
        &full(|_| {}),
        None,
        &[fdi(32)],
        None,
        &BTreeMap::new(),
    );
    let mv = moved(&res, &a.cusps);
    assert_eq!(mv[&fdi(32)], 0.0);
    assert!(mv[&fdi(42)] > 0.5);
}

fn with_poses(a: &Synth, cfg: &Config, poses: &BTreeMap<Fdi, DMat4>) -> SetupResult {
    conservative_setup(&a.teeth, &seq(), &a.cusps, cfg, None, &[], None, poses)
}

#[test]
fn a_tooth_locked_by_hand_is_the_reference() {
    let a = regular();
    let d = DMat4::from_translation(a.teeth[&fdi(42)].e_md);
    let res = with_poses(&a, &full(|_| {}), &BTreeMap::from([(fdi(42), d)]));
    assert!(!res.targets.contains_key(&fdi(42)));
    assert!(res.held.contains(&fdi(42)));
    let mv = moved(&res, &a.cusps);
    assert!(mv[&fdi(41)] > 0.5);
    assert!(res.overlap_max <= 0.16);
    for f in [47, 46, 36, 37] {
        assert_eq!(mv[&fdi(f)], 0.0, "{f}");
    }
}

#[test]
fn a_locked_tooth_with_no_move_stays() {
    let a = make_arch(&Mal {
        shift: vec![(32, DVec3::new(0.0, -1.2, 0.0))],
        ..Mal::default()
    });
    let res = with_poses(
        &a,
        &full(|_| {}),
        &BTreeMap::from([(fdi(32), DMat4::IDENTITY)]),
    );
    assert!(!res.targets.contains_key(&fdi(32)));
}

#[test]
fn a_rotation_below_the_threshold_is_not_corrected() {
    let a = make_arch(&Mal {
        rot: vec![(44, 3.0)],
        ..Mal::default()
    });
    let res = setup(&a, &full(|c| c.rot_tol_deg = 5.0));
    assert!(!res.targets.contains_key(&fdi(44)));
}

#[test]
fn a_rotation_above_the_threshold_is_corrected_by_the_excess() {
    let a = make_arch(&Mal {
        rot: vec![(42, 12.0)],
        ..Mal::default()
    });
    // tolerance 5 degrees: the plugin turns 42 fully onto the line, the port 5 degrees less
    let plugin = setup(
        &a,
        &full(|c| {
            c.rot_tol_deg = 5.0;
            c.plugin_derotation = true;
        }),
    );
    let full_turn = angle(&plugin.targets[&fdi(42)]);
    assert!(full_turn > 8.0 && full_turn < 13.0, "{full_turn}");
    let res = setup(&a, &full(|c| c.rot_tol_deg = 5.0));
    let ang = angle(&res.targets[&fdi(42)]);
    assert!(
        (full_turn - ang - 5.0).abs() < 0.1,
        "{ang} against {full_turn}"
    );
}

#[test]
fn the_derotation_grows_from_nothing_beyond_the_tolerance() {
    // tolerance 1 degree: the plugin turns a tooth fully onto the line, the port by the
    // excess over the tolerance, and not at all under half a degree of excess
    for rot in [1.2, 2.5, 4.0, 8.0] {
        let a = make_arch(&Mal {
            rot: vec![(42, rot)],
            ..Mal::default()
        });
        let plugin = setup(&a, &full(|c| c.plugin_derotation = true));
        let full_turn = plugin.targets.get(&fdi(42)).map_or(0.0, angle);
        let res = setup(&a, &full(|_| {}));
        let turn = res.targets.get(&fdi(42)).map_or(0.0, angle);
        if full_turn - 1.0 < MIN_DEROTATION_DEG - 0.1 {
            assert_eq!(turn, 0.0, "{rot}: {turn} against {full_turn}");
        } else {
            assert!(
                (full_turn - turn - 1.0).abs() < 0.1,
                "{rot}: {turn} against {full_turn}"
            );
        }
    }
}

#[test]
fn crowding_without_space_uses_ipr() {
    let a = crowded(&[(42, 6.3), (41, 5.8), (31, 5.8), (32, 6.3)]);
    let res = setup(&a, &full(|c| c.fix_canines = true));
    assert!(res.ipr.values().sum::<f64>() > 1.0);
    for (pair, mm) in &res.ipr {
        assert!(*mm <= 0.5 + 1e-9, "{pair:?}");
    }
    assert!(res.overlap_max <= 0.16);
}

#[test]
fn a_drawn_line_is_followed() {
    let a = regular();
    let l = cusp_line(&a.cusps, &seq(), 5.0, &[], false);
    let n = line_normals(&l, interior(&a.cusps, &seq()));
    let l2: Vec<DVec3> = l
        .iter()
        .zip(&n)
        .map(|(p, nn)| *p + (0.8 * *nn).extend(0.0))
        .collect();
    let res = conservative_setup(
        &a.teeth,
        &seq(),
        &a.cusps,
        &full(|_| {}),
        Some(&l2),
        &[],
        None,
        &BTreeMap::new(),
    );
    let moved_cusps = after(&res, &a.cusps);
    for f in [45, 44, 43, 33, 34, 35] {
        assert!(max_dist(&l2, &moved_cusps[&fdi(f)]) < 0.35, "{f}");
    }
}

// ---- complete alignment -------------------------------------------------------------------------------

#[test]
fn a_small_offset_from_the_line_is_corrected() {
    let a = make_arch(&Mal {
        shift: vec![(44, DVec3::new(0.0, -0.25, 0.0))],
        ..Mal::default()
    });
    let res = setup(&a, &full(|_| {}));
    assert!(res.targets.contains_key(&fdi(44)));
    assert!(max_dist(&res.line, &after(&res, &a.cusps)[&fdi(44)]) < 0.1);
}

#[test]
fn a_small_rotation_is_corrected_by_the_excess() {
    let a = make_arch(&Mal {
        rot: vec![(44, 3.0)],
        ..Mal::default()
    });
    // 3 degrees, tolerance 1: turned by 2
    let res = setup(&a, &full(|_| {}));
    let ang = angle(&res.targets[&fdi(44)]);
    assert!((ang - 2.0).abs() < 0.3, "{ang}");
}

#[test]
fn a_rotated_incisor_is_derotated_with_the_space() {
    let a = make_arch(&Mal {
        rot: vec![(42, 25.0)],
        ..Mal::default()
    });
    let res = setup(
        &a,
        &full(|c| {
            c.expansion_posterior = 0.0;
            c.advance_max = 0.0;
        }),
    );
    assert!(angle(&res.targets[&fdi(42)]) > 20.0);
    assert!(res.overlap_max <= 0.16);
    assert!(max_dist(&res.line, &after(&res, &a.cusps)[&fdi(42)]) < 0.15);
}

#[test]
fn severe_crowding_does_not_push_on_the_molars() {
    let a = crowded(&[
        (43, 8.8),
        (42, 7.5),
        (41, 7.0),
        (31, 7.0),
        (32, 7.5),
        (33, 8.8),
    ]);
    let res = setup(
        &a,
        &full(|c| {
            c.expansion_posterior = 0.0;
            c.advance_max = 0.0;
        }),
    );
    let mv = moved(&res, &a.cusps);
    for f in [47, 46, 36, 37] {
        assert!(mv[&fdi(f)] < 0.05, "{f}");
    }
    assert!(res.residual > 3.0);
    assert!(has(&res, |n| matches!(n, Note::UnresolvedCrowding { .. })));
    assert!(mv[&fdi(45)] < 1.5);
    assert!(mv[&fdi(35)] < 1.5);
}

#[test]
fn the_order_of_the_therapy() {
    let a = crowded(&[(42, 6.3), (41, 5.8), (31, 5.8), (32, 6.3)]);
    let ipr_first = setup(&a, &full(|_| {}));
    assert!(ipr_first.ipr.values().sum::<f64>() > 1.0);
    assert_eq!(ipr_first.expansion, 0.0);
    let exp_first = setup(
        &a,
        &full(|c| {
            c.priority = vec![Therapy::Exp, Therapy::Ipr, Therapy::Pro];
            c.expansion_canine = 2.0;
        }),
    );
    assert!(exp_first.expansion > 0.0);
    assert!(exp_first.ipr.values().sum::<f64>() < ipr_first.ipr.values().sum::<f64>());
    for res in [&ipr_first, &exp_first] {
        assert!(res.overlap_max <= 0.16);
    }
}

#[test]
fn incisor_advancement() {
    let a = crowded(&[(42, 6.3), (41, 5.8), (31, 5.8), (32, 6.3)]);
    let res = setup(
        &a,
        &full(|c| {
            c.priority = vec![Therapy::Pro, Therapy::Ipr, Therapy::Exp];
            c.advance_max = 2.0;
        }),
    );
    assert!(res.advance > 0.1);
    assert!(res.overlap_max <= 0.16);
}

#[test]
fn a_diastema_is_closed() {
    let a = crowded(&[(41, 4.65), (31, 4.65)]);
    let gaps = |res: Option<&SetupResult>| {
        let mut out = 0.0;
        for (fa, ma, fb, mb) in [
            (42, true, 41, false),
            (41, true, 31, true),
            (31, false, 32, true),
        ] {
            let pt = |f: u8, mesial: bool| {
                let t = &a.teeth[&fdi(f)];
                let p = if mesial { t.contact_m } else { t.contact_d };
                res.and_then(|r| r.targets.get(&fdi(f)))
                    .map_or(p, |d| d.transform_point3(p))
            };
            out += (pt(fa, ma) - pt(fb, mb)).truncate().length();
        }
        out
    };
    let before = gaps(None);
    let closed = setup(&a, &full(|c| c.retract_max = 0.0));
    let kept = setup(&a, &full(|c| c.close_spaces = false));
    assert!(before > 1.3);
    assert!(gaps(Some(&closed)) < 0.3, "{}", gaps(Some(&closed)));
    assert!(gaps(Some(&kept)) > before - 0.2);
    let retracted = setup(&a, &full(|_| {}));
    assert!(retracted.advance < 0.0);
}

#[test]
fn levelling() {
    let a = make_arch(&Mal {
        shift: vec![(44, DVec3::new(0.0, 0.0, 0.6))],
        ..Mal::default()
    });
    let res = setup(&a, &full(|_| {}));
    assert!(mean_z(&after(&res, &a.cusps)[&fdi(44)]).abs() < 0.35);
    let none = setup(&a, &full(|c| c.leveling = Leveling::None));
    assert!((mean_z(&after(&none, &a.cusps)[&fdi(44)]) - 0.6).abs() < 1e-6);
}

// ---- anterior teeth -----------------------------------------------------------------------------------

fn long_canines(extra: &[(u8, DVec3)]) -> Synth {
    let mut shift = vec![(43, DVec3::Z), (33, DVec3::Z)];
    shift.extend_from_slice(extra);
    make_arch(&Mal {
        shift,
        ..Mal::default()
    })
}

#[test]
fn the_canines_keep_their_height() {
    let a = long_canines(&[(42, DVec3::new(0.0, -1.0, 0.0))]);
    let res = setup(&a, &Config::default());
    let moved_cusps = after(&res, &a.cusps);
    for f in [43, 33] {
        assert!((mean_z(&moved_cusps[&fdi(f)]) - 1.0).abs() < 1e-6, "{f}");
    }
}

#[test]
fn premolars_are_not_raised_towards_the_canines() {
    let a = long_canines(&[]);
    for cfg in [
        Config::default(),
        Config {
            align_scope: AlignScope::All,
            ..Config::default()
        },
    ] {
        let moved_cusps = after(&setup(&a, &cfg), &a.cusps);
        for f in [44, 45, 34, 35, 42, 32] {
            assert!(
                mean_z(&moved_cusps[&fdi(f)]).abs() < 1e-6,
                "{:?} {f}",
                cfg.align_scope
            );
        }
    }
}

#[test]
fn premolars_and_molars_stay() {
    let a = make_arch(&Mal {
        shift: vec![
            (44, DVec3::new(0.0, -0.8, 0.0)),
            (32, DVec3::new(0.0, -1.0, 0.0)),
        ],
        rot: vec![(46, 6.0)],
        ..Mal::default()
    });
    let res = setup(&a, &Config::default());
    let mv = moved(&res, &a.cusps);
    for f in seq() {
        if anatomy::index(f) >= 4 {
            assert_eq!(mv[&f], 0.0, "{f}");
        }
    }
    assert!(mv[&fdi(32)] > 0.5);
    assert_eq!(res.expansion, 0.0);
    let premolars = setup(
        &a,
        &Config {
            align_scope: AlignScope::Premolars,
            ..Config::default()
        },
    );
    let mv = moved(&premolars, &a.cusps);
    assert!(mv[&fdi(44)] > 0.5);
    assert_eq!(mv[&fdi(46)], 0.0);
}

// ---- the spaces of the arch (2026-11-24) -------------------------------------------------------------

/// Gap along the arch of each contact (mm, negative: overlap) between the
/// contact points that face each other, at rest (`None`) or moved by the
/// targets of `res`.
fn contact_gaps(a: &Synth, res: Option<&SetupResult>) -> Vec<(u8, u8, f64)> {
    let mv = |f: Fdi, p: DVec3| {
        res.and_then(|r| r.targets.get(&f))
            .map_or(p, |d| d.transform_point3(p))
    };
    seq()
        .windows(2)
        .map(|w| {
            let pt = |f: Fdi, g: Fdi| {
                let t = &a.teeth[&f];
                let p = match facing_surface(f, g) {
                    Surface::Mesial => t.contact_m,
                    _ => t.contact_d,
                };
                mv(f, p)
            };
            let (pa, pb) = (pt(w[0], w[1]), pt(w[1], w[0]));
            let (ca, cb) = (
                mv(w[0], mean(&a.cusps[&w[0]])),
                mv(w[1], mean(&a.cusps[&w[1]])),
            );
            let u = (cb - ca).truncate().normalize();
            (w[0].get(), w[1].get(), (pb - pa).truncate().dot(u))
        })
        .collect()
}

/// Movement of each tooth towards the midline along the arch (mm).
fn mesial_moves(a: &Synth, res: &SetupResult) -> BTreeMap<u8, f64> {
    seq()
        .iter()
        .map(|f| {
            let c = mean(&a.cusps[f]);
            let d = res.targets.get(f).map_or(c, |d| d.transform_point3(c)) - c;
            (f.get(), d.truncate().dot(a.teeth[f].e_md.truncate()))
        })
        .collect()
}

/// The synthetic arch with spaces (`gaps`) and the teeth of `widths` wider
/// or narrower where they are (overlapping their neighbours, or apart).
fn spaced(gaps: &[(u8, f64)], widths: &[(u8, f64)]) -> Synth {
    let mut a = make_arch(&Mal {
        gaps: gaps.to_vec(),
        ..Mal::default()
    });
    for &(f, w) in widths {
        let c = mean(&a.cusps[&fdi(f)]);
        let t = a.teeth.get_mut(&fdi(f)).unwrap();
        t.width_md = w;
        t.contact_m = c + t.e_md * w / 2.0;
        t.contact_d = c - t.e_md * w / 2.0;
    }
    a
}

#[test]
fn spaces_between_canines_and_premolars_close_with_the_posterior_teeth() {
    // 1.2 mm between each canine and its first premolar, every other contact closed
    let a = spaced(&[(34, 1.2), (44, 1.2)], &[]);
    for (x, y, g) in contact_gaps(&a, None) {
        let want = if [(44, 43), (33, 34)].contains(&(x, y)) {
            1.2
        } else {
            0.0
        };
        assert!((g - want).abs() < 0.1, "at rest {x}-{y}: {g:.2} mm");
    }
    let res = setup(&a, &Params::default().config(Arch::Lower));
    for (x, y, g) in contact_gaps(&a, Some(&res)) {
        assert!(g.abs() < 0.2, "final {x}-{y}: {g:.2} mm");
    }
    // the premolars and the molars go mesially to close them (none distally), the space is
    // not stripped
    let along = mesial_moves(&a, &res);
    for f in [44, 45, 46, 47, 34, 35, 36, 37] {
        assert!(along[&f] > 0.8, "{f}: {:.2} mm", along[&f]);
    }
    assert!(along.values().all(|&d| d > -0.1), "{along:?}");
    assert!(res.ipr.is_empty(), "{:?}", res.ipr);
}

#[test]
fn no_ipr_while_the_arch_has_spaces() {
    // the lower incisors 1.2 mm too wide together, 2 mm of space between the second premolars
    // and the first molars: the space covers the crowding, no enamel is removed
    let a = spaced(
        &[(36, 1.0), (46, 1.0)],
        &[(41, 5.7), (42, 6.2), (31, 5.7), (32, 6.2)],
    );
    let res = setup(&a, &Params::default().config(Arch::Lower));
    assert!(res.ipr.is_empty(), "{:?}", res.ipr);
    assert!(res.residual < 1e-6, "{}", res.residual);
    for (x, y, g) in contact_gaps(&a, Some(&res)) {
        assert!(g.abs() < 0.2, "final {x}-{y}: {g:.2} mm");
    }
}

#[test]
fn no_ipr_on_a_contact_open_at_rest() {
    // 2.4 mm of crowding in the incisors and 0.5 mm of space between 33 and 34: IPR makes the
    // rest, at the closed contacts only, and the space closes
    let a = spaced(&[(34, 0.5)], &[(41, 6.0), (42, 6.5), (31, 6.0), (32, 6.5)]);
    let res = setup(&a, &Params::default().config(Arch::Lower));
    assert!(!res.ipr.is_empty());
    assert!(!res.ipr.contains_key(&(fdi(33), fdi(34))), "{:?}", res.ipr);
    let g = contact_gaps(&a, Some(&res));
    let g34 = g.iter().find(|x| (x.0, x.1) == (33, 34)).unwrap().2;
    assert!(g34.abs() < 0.2, "{g34:.2} mm");
}

#[test]
fn a_small_step_is_tolerated_a_large_one_reduced() {
    let a = make_arch(&Mal {
        shift: vec![
            (42, DVec3::new(0.0, 0.0, 0.4)),
            (32, DVec3::new(0.0, 0.0, 1.2)),
        ],
        ..Mal::default()
    });
    let moved_cusps = after(&setup(&a, &Config::default()), &a.cusps);
    assert!((mean_z(&moved_cusps[&fdi(42)]) - 0.4).abs() < 1e-6);
    let z = mean_z(&moved_cusps[&fdi(32)]);
    assert!(z > 0.35 && z < 0.75, "{z}");
}

// ---- realistic alignment (crowns) ---------------------------------------------------------------------

/// `_crown`: a synthetic incisor or canine crown (a slab `width_md` wide
/// along the true mesio-distal direction, 6 mm thick at the neck and 0.8 at
/// the incisal edge, on top at the height of the cusps), rotated by `rot`
/// degrees about the vertical through the centre of the cusps.
fn synthetic_crown(a: &Synth, f: u8, rot: f64) -> Vec<DVec3> {
    let height = 9.0;
    let g = &a.teeth[&fdi(f)];
    let c = mean(&a.cusps[&fdi(f)]);
    let m = g.e_md;
    let mut out = DVec3::new(-m.y, m.x, 0.0);
    if out.dot(DVec3::new(c.x, c.y, 0.0)) < 0.0 {
        out = -out;
    }
    // numpy.linspace
    let lin = |a: f64, b: f64, n: usize| -> Vec<f64> {
        let step = (b - a) / (n - 1) as f64;
        (0..n)
            .map(|i| if i == n - 1 { b } else { i as f64 * step + a })
            .collect()
    };
    let mut pts = Vec::new();
    for z in super::line::arange(0.0, height + 1e-9, 0.25) {
        let half = 3.0 * (z / height) + 0.4;
        for u in lin(-g.width_md / 2.0, g.width_md / 2.0, 25) {
            for side in [-1.0, 1.0] {
                pts.push(c + m * u + out * side * half - DVec3::new(0.0, 0.0, z));
            }
        }
        for v in lin(-half, half, 7) {
            for u in [-g.width_md / 2.0, g.width_md / 2.0] {
                pts.push(c + m * u + out * v - DVec3::new(0.0, 0.0, z));
            }
        }
    }
    pts.iter()
        .map(|&q| c + rotz(q - c, rot.to_radians()))
        .collect()
}

/// Angle (degrees) between the incisal edge of the crown (1 mm band on top)
/// and the line.
fn edge_angle(p: &[DVec3], l: &[DVec3]) -> f64 {
    let zmax = p.iter().map(|q| q.z).fold(f64::NEG_INFINITY, f64::max);
    let top: Vec<DVec2> = p
        .iter()
        .filter(|q| q.z >= zmax - 1.0)
        .map(|q| q.truncate())
        .collect();
    let m = top.iter().copied().sum::<DVec2>() / top.len() as f64;
    let (mut a, mut b, mut c) = (0.0, 0.0, 0.0);
    for q in &top {
        let d = *q - m;
        a += d.x * d.x;
        b += d.x * d.y;
        c += d.y * d.y;
    }
    let eig = nalgebra::SymmetricEigen::new(nalgebra::Matrix2::new(a, b, b, c));
    let k = if eig.eigenvalues[0] > eig.eigenvalues[1] {
        0
    } else {
        1
    };
    let e = DVec2::new(eig.eigenvectors[(0, k)], eig.eigenvectors[(1, k)]);
    let sl = arc(l);
    let centre = mean(p);
    let tg = tangent_at(
        l,
        &sl,
        project(l, &sl, DVec3::new(centre.x, centre.y, 0.0)).0,
    );
    e.dot(tg).abs().min(1.0).acos().to_degrees()
}

#[test]
fn the_axis_comes_from_the_crown_when_the_segmentation_is_wrong() {
    let mut a = make_arch(&Mal {
        rot: vec![(41, 20.0)],
        ..Mal::default()
    });
    let straight = regular();
    a.teeth.get_mut(&fdi(41)).unwrap().e_md = straight.teeth[&fdi(41)].e_md;
    a.cusps.insert(fdi(41), straight.cusps[&fdi(41)].clone());
    let crowns: BTreeMap<Fdi, Vec<DVec3>> = [42, 41, 31, 32]
        .into_iter()
        .map(|f| {
            (
                fdi(f),
                synthetic_crown(&straight, f, if f == 41 { 20.0 } else { 0.0 }),
            )
        })
        .collect();
    let cfg = Config {
        ipr_zone: IprZone::AnteriorPremolars,
        ..Config::default()
    };
    let with_crowns = conservative_setup(
        &a.teeth,
        &seq(),
        &a.cusps,
        &cfg,
        None,
        &[],
        Some(&crowns),
        &BTreeMap::new(),
    );
    let d = with_crowns.targets[&fdi(41)];
    let moved: Vec<DVec3> = crowns[&fdi(41)]
        .iter()
        .map(|&p| d.transform_point3(p))
        .collect();
    assert!(edge_angle(&moved, &with_crowns.line) < 3.0);
    let without = setup(&a, &cfg);
    let d0 = without
        .targets
        .get(&fdi(41))
        .copied()
        .unwrap_or(DMat4::IDENTITY);
    let moved0: Vec<DVec3> = crowns[&fdi(41)]
        .iter()
        .map(|&p| d0.transform_point3(p))
        .collect();
    assert!(edge_angle(&moved0, &without.line) > 10.0);
}

#[test]
fn not_enough_space_partial_derotation_canine_first() {
    let mut a = make_arch(&Mal {
        rot: vec![(42, 30.0), (43, 15.0)],
        ..Mal::default()
    });
    for (f, deg) in [(42u8, 30.0f64), (43, 15.0)] {
        let c = mean(&a.cusps[&fdi(f)]);
        let g = a.teeth.get_mut(&fdi(f)).unwrap();
        let w = width_of(f % 10) / deg.to_radians().cos();
        g.width_md = w;
        g.contact_m = c + g.e_md * w / 2.0;
        g.contact_d = c - g.e_md * w / 2.0;
    }
    let cfg = Config {
        ipr_surface_cap: 0.05,
        advance_max: 0.0,
        expansion_canine: 0.0,
        ..Config::default()
    };
    let res = setup(&a, &cfg);
    assert!(res.overlap_max <= 0.16);
    assert!(has(&res, |n| matches!(n, Note::PartialDerotation { .. })));
    let frac = |f: u8, deg: f64| res.targets.get(&fdi(f)).map_or(0.0, |d| angle(d) / deg);
    assert!(frac(42, 30.0) > frac(43, 15.0));
    assert!(frac(42, 30.0) < 1.0);
}

#[test]
fn no_advancement_with_the_incisors_locked() {
    let a = crowded(&[(43, 6.0), (33, 6.0)]);
    let poses: BTreeMap<Fdi, DMat4> = [42, 41, 31, 32]
        .into_iter()
        .map(|f| (fdi(f), DMat4::IDENTITY))
        .collect();
    let res = with_poses(&a, &Config::default(), &poses);
    assert_eq!(res.advance, 0.0);
    for f in [43, 33] {
        let d = res.targets.get(&fdi(f)).copied().unwrap_or(DMat4::IDENTITY);
        let c0 = mean(&a.cusps[&fdi(f)]);
        let inward = (d.transform_point3(c0) - c0)
            .truncate()
            .dot(-c0.truncate() / c0.truncate().length());
        assert!(inward < 0.2, "{f}");
    }
}

// ---- evaluation ---------------------------------------------------------------------------------------

#[test]
fn only_the_tooth_that_needs_it_moves() {
    let lingual = -0.2 * regular().out_dir[&fdi(45)];
    let a = make_arch(&Mal {
        shift: vec![(42, DVec3::new(0.0, -1.2, 0.0)), (45, lingual)],
        rot: vec![(46, 3.0)],
        ..Mal::default()
    });
    let res = setup(
        &a,
        &Config {
            align_scope: AlignScope::All,
            ..Config::default()
        },
    );
    let mv = moved(&res, &a.cusps);
    assert!(mv[&fdi(42)] > 0.6);
    for f in [45, 46, 47, 44, 34, 35, 36, 37] {
        assert!(mv[&fdi(f)] < 0.15, "{f}");
    }
    let Some(Note::Evaluation { to_align, in_place }) = res
        .log
        .iter()
        .find(|n| matches!(n, Note::Evaluation { .. }))
    else {
        panic!("no evaluation");
    };
    assert!(to_align.iter().any(|d| d.fdi == fdi(42)));
    for f in [45, 46, 47] {
        assert!(!to_align.iter().any(|d| d.fdi == fdi(f)), "{f}");
        assert!(in_place.contains(&fdi(f)), "{f}");
    }
}

#[test]
fn teeth_in_contact_stay_in_contact() {
    let mut a = regular();
    let crowns: BTreeMap<Fdi, Vec<DVec3>> = [46, 45, 44, 43, 33, 34, 35, 36]
        .into_iter()
        .map(|f| (fdi(f), synthetic_crown(&a, f, 0.0)))
        .collect();
    for f in [45, 44, 34, 35] {
        let c = mean(&a.cusps[&fdi(f)]);
        let g = a.teeth.get_mut(&fdi(f)).unwrap();
        let w = g.width_md + 0.8;
        g.contact_m = c + g.e_md * w / 2.0;
        g.contact_d = c - g.e_md * w / 2.0;
    }
    let cfg = Config {
        align_scope: AlignScope::All,
        ..Config::default()
    };
    let with_crowns = conservative_setup(
        &a.teeth,
        &seq(),
        &a.cusps,
        &cfg,
        None,
        &[],
        Some(&crowns),
        &BTreeMap::new(),
    );
    assert!(
        moved(&with_crowns, &a.cusps)
            .values()
            .copied()
            .fold(0.0, f64::max)
            < 0.15
    );
    assert!(with_crowns.ipr.is_empty());
    assert!(!has(&with_crowns, |n| matches!(
        n,
        Note::UnresolvedCrowding { .. }
    )));
    let without = setup(&a, &cfg);
    assert!(
        !without.ipr.is_empty()
            || moved(&without, &a.cusps)
                .values()
                .copied()
                .fold(0.0, f64::max)
                > 0.3
    );
}

#[test]
fn a_crooked_locked_tooth_is_reported() {
    let a = make_arch(&Mal {
        shift: vec![(42, DVec3::new(0.0, -1.2, 0.0))],
        ..Mal::default()
    });
    let res = with_poses(
        &a,
        &Config::default(),
        &BTreeMap::from([(fdi(42), DMat4::IDENTITY)]),
    );
    assert!(!res.targets.contains_key(&fdi(42)));
    assert!(has(
        &res,
        |n| matches!(n, Note::LockedOffLine { tooth } if tooth.fdi == fdi(42))
    ));
}

#[test]
fn expansion_moves_the_teeth_in_place_too() {
    let a = crowded(&[(42, 6.3), (41, 5.8), (31, 5.8), (32, 6.3)]);
    let cfg = Config {
        align_scope: AlignScope::All,
        priority: vec![Therapy::Exp, Therapy::Ipr, Therapy::Pro],
        expansion_posterior: 2.0,
        expansion_canine: 1.0,
        advance_max: 0.0,
        ipr_zone: IprZone::None,
        ..Config::default()
    };
    let res = setup(&a, &cfg);
    assert!(res.expansion > 0.1);
    let d = res.targets.get(&fdi(45)).expect("45 moves with the line");
    let c0 = mean(&a.cusps[&fdi(45)]);
    assert!((d.transform_point3(c0) - c0).dot(a.out_dir[&fdi(45)]) > 0.1);
}

// ---- determinism and options --------------------------------------------------------------------------

#[test]
fn the_same_input_gives_the_same_setup() {
    let a = crowded(&[(42, 6.3), (41, 5.8), (31, 5.8), (32, 6.3)]);
    let crowns: BTreeMap<Fdi, Vec<DVec3>> = [42, 41, 31, 32]
        .into_iter()
        .map(|f| (fdi(f), synthetic_crown(&a, f, 0.0)))
        .collect();
    let run = || {
        conservative_setup(
            &a.teeth,
            &seq(),
            &a.cusps,
            &Config::default(),
            None,
            &[],
            Some(&crowns),
            &BTreeMap::new(),
        )
    };
    assert_eq!(run(), run());
}

#[test]
fn the_midline_can_go_to_the_centre() {
    // the plugin rebinds `k` inside `solve` for this option (and then fails when a tooth
    // needs a derotation); the port keeps the derotation fractions apart
    let a = make_arch(&Mal {
        shift: vec![
            (41, DVec3::new(1.0, 0.0, 0.0)),
            (31, DVec3::new(1.0, 0.0, 0.0)),
        ],
        rot: vec![(42, 8.0)],
        ..Mal::default()
    });
    let res = setup(
        &a,
        &Config {
            midline: Midline::Center,
            ..Config::default()
        },
    );
    assert!(res.targets.contains_key(&fdi(42)));
    assert!(res.midline_shift < -0.3, "{}", res.midline_shift);
}

#[test]
fn the_default_parameters_are_the_plugin_scene_settings_but_the_scope_and_the_molars() {
    let p = Params::default();
    let lower = p.config(Arch::Lower);
    assert_eq!(lower.ipr_zone, IprZone::AnteriorPremolars);
    assert_eq!(
        (lower.expansion_posterior, lower.expansion_canine),
        (2.0, 0.0)
    );
    let upper = p.config(Arch::Upper);
    assert_eq!(
        (upper.expansion_posterior, upper.expansion_canine),
        (2.0, 2.0)
    );
    // every tooth aligned and no molar held (2026-11-24), the plugin's dataclass otherwise
    assert_eq!(
        (lower.align_scope, lower.fix_molars),
        (AlignScope::All, false)
    );
    let base = Config::default();
    assert_eq!(
        Config {
            ipr_zone: IprZone::Anterior,
            align_scope: AlignScope::Anterior,
            fix_molars: true,
            ..lower
        },
        base
    );
    let back: Params = serde_json::from_str("{}").unwrap();
    assert_eq!(back, p);
}

#[test]
fn facing_surfaces() {
    assert_eq!(facing_surface(fdi(41), fdi(31)), Surface::Mesial);
    assert_eq!(facing_surface(fdi(42), fdi(41)), Surface::Mesial);
    assert_eq!(facing_surface(fdi(41), fdi(42)), Surface::Distal);
}

// ---- the options against the plugin ---------------------------------------------------------------

/// The scenarios of `tests/fixtures/gen_setup.py`, built with the same helpers.
fn scenarios() -> Vec<(&'static str, SetupResult)> {
    let mut out = Vec::new();
    let mut add = |name: &'static str,
                   a: &Synth,
                   cfg: Config,
                   line: Option<Vec<DVec3>>,
                   fixed: &[u8],
                   crowns: Option<BTreeMap<Fdi, Vec<DVec3>>>,
                   poses: BTreeMap<Fdi, DMat4>| {
        let fixed: Vec<Fdi> = fixed.iter().map(|&n| fdi(n)).collect();
        // the plugin derotates fully, and leaves the spaces as it does
        let cfg = Config {
            plugin_derotation: true,
            plugin_line_scale: true,
            plugin_spaces: true,
            ..cfg
        };
        let r = conservative_setup(
            &a.teeth,
            &seq(),
            &a.cusps,
            &cfg,
            line.as_deref(),
            &fixed,
            crowns.as_ref(),
            &poses,
        );
        out.push((name, r));
    };
    let none = BTreeMap::new;
    let shift = |v: &[(u8, [f64; 3])]| -> Mal {
        Mal {
            shift: v.iter().map(|(f, d)| (*f, DVec3::from(*d))).collect(),
            ..Mal::default()
        }
    };

    let a = make_arch(&shift(&[(32, [0.0, -1.2, 0.0])]));
    add("shifted", &a, full(|_| {}), None, &[], None, none());
    let a = make_arch(&shift(&[(32, [0.0, -1.2, 0.0]), (42, [0.0, -1.2, 0.0])]));
    add("excluded", &a, full(|_| {}), None, &[32], None, none());
    let a = regular();
    let d = DMat4::from_translation(a.teeth[&fdi(42)].e_md * 1.0);
    add(
        "locked_moved",
        &a,
        full(|_| {}),
        None,
        &[],
        None,
        BTreeMap::from([(fdi(42), d)]),
    );
    let a = make_arch(&shift(&[(42, [0.0, -1.2, 0.0])]));
    add(
        "locked_crooked",
        &a,
        Config::default(),
        None,
        &[],
        None,
        BTreeMap::from([(fdi(42), DMat4::IDENTITY)]),
    );
    let a = regular();
    let l = cusp_line(&a.cusps, &seq(), 5.0, &[], true);
    let n = line_normals(&l, interior(&a.cusps, &seq()));
    let l2: Vec<DVec3> = l
        .iter()
        .zip(&n)
        .map(|(p, nn)| *p + (0.8 * *nn).extend(0.0))
        .collect();
    add("drawn_line", &a, full(|_| {}), Some(l2), &[], None, none());
    let a = crowded(&[(42, 6.3), (41, 5.8), (31, 5.8), (32, 6.3)]);
    add(
        "ipr_fixed_canines",
        &a,
        full(|c| c.fix_canines = true),
        None,
        &[],
        None,
        none(),
    );
    add(
        "exp_first",
        &a,
        full(|c| {
            c.priority = vec![Therapy::Exp, Therapy::Ipr, Therapy::Pro];
            c.expansion_canine = 2.0;
        }),
        None,
        &[],
        None,
        none(),
    );
    add(
        "pro_first",
        &a,
        full(|c| {
            c.priority = vec![Therapy::Pro, Therapy::Ipr, Therapy::Exp];
            c.advance_max = 2.0;
        }),
        None,
        &[],
        None,
        none(),
    );
    add(
        "expansion_in_place",
        &a,
        Config {
            align_scope: AlignScope::All,
            priority: vec![Therapy::Exp, Therapy::Ipr, Therapy::Pro],
            expansion_posterior: 2.0,
            expansion_canine: 1.0,
            advance_max: 0.0,
            ipr_zone: IprZone::None,
            ..Config::default()
        },
        None,
        &[],
        None,
        none(),
    );
    let a = crowded(&[(41, 4.65), (31, 4.65)]);
    add(
        "diastema_retract",
        &a,
        full(|_| {}),
        None,
        &[],
        None,
        none(),
    );
    add(
        "diastema_open",
        &a,
        full(|c| c.close_spaces = false),
        None,
        &[],
        None,
        none(),
    );
    let a = crowded(&[
        (43, 8.8),
        (42, 7.5),
        (41, 7.0),
        (31, 7.0),
        (32, 7.5),
        (33, 8.8),
    ]);
    add(
        "severe",
        &a,
        full(|c| {
            c.expansion_posterior = 0.0;
            c.advance_max = 0.0;
        }),
        None,
        &[],
        None,
        none(),
    );
    let mut a = make_arch(&Mal {
        rot: vec![(42, 30.0), (43, 15.0)],
        ..Mal::default()
    });
    for (f, deg) in [(42u8, 30.0f64), (43, 15.0)] {
        let c = mean(&a.cusps[&fdi(f)]);
        let g = a.teeth.get_mut(&fdi(f)).unwrap();
        let w = width_of(f % 10) / deg.to_radians().cos();
        g.width_md = w;
        g.contact_m = c + g.e_md * w / 2.0;
        g.contact_d = c - g.e_md * w / 2.0;
    }
    add(
        "partial_derotation",
        &a,
        Config {
            ipr_surface_cap: 0.05,
            advance_max: 0.0,
            expansion_canine: 0.0,
            ..Config::default()
        },
        None,
        &[],
        None,
        none(),
    );
    let mut a = make_arch(&Mal {
        rot: vec![(41, 20.0)],
        ..Mal::default()
    });
    let straight = regular();
    a.teeth.get_mut(&fdi(41)).unwrap().e_md = straight.teeth[&fdi(41)].e_md;
    a.cusps.insert(fdi(41), straight.cusps[&fdi(41)].clone());
    let crowns: BTreeMap<Fdi, Vec<DVec3>> = [42, 41, 31, 32]
        .into_iter()
        .map(|f| {
            (
                fdi(f),
                synthetic_crown(&straight, f, if f == 41 { 20.0 } else { 0.0 }),
            )
        })
        .collect();
    add(
        "crown_axis",
        &a,
        Config {
            ipr_zone: IprZone::AnteriorPremolars,
            ..Config::default()
        },
        None,
        &[],
        Some(crowns),
        none(),
    );
    let mut a = regular();
    let crowns: BTreeMap<Fdi, Vec<DVec3>> = [46, 45, 44, 43, 33, 34, 35, 36]
        .into_iter()
        .map(|f| (fdi(f), synthetic_crown(&a, f, 0.0)))
        .collect();
    for f in [45, 44, 34, 35] {
        let c = mean(&a.cusps[&fdi(f)]);
        let g = a.teeth.get_mut(&fdi(f)).unwrap();
        let w = g.width_md + 0.8;
        g.contact_m = c + g.e_md * w / 2.0;
        g.contact_d = c - g.e_md * w / 2.0;
    }
    add(
        "crown_contacts",
        &a,
        Config {
            align_scope: AlignScope::All,
            ..Config::default()
        },
        None,
        &[],
        Some(crowns),
        none(),
    );
    let a = make_arch(&shift(&[(44, [0.0, 0.0, 0.6]), (32, [0.0, -0.6, 0.4])]));
    add(
        "flat_leveling",
        &a,
        full(|c| c.leveling = Leveling::Flat),
        None,
        &[],
        None,
        none(),
    );
    let a = make_arch(&shift(&[
        (43, [0.0, 0.0, 1.0]),
        (33, [0.0, 0.0, 1.0]),
        (42, [0.0, -1.0, 0.0]),
    ]));
    add(
        "level_canines",
        &a,
        Config {
            align_scope: AlignScope::All,
            level_canines: true,
            ..Config::default()
        },
        None,
        &[],
        None,
        none(),
    );
    let a = make_arch(&Mal {
        shift: vec![
            (44, DVec3::new(0.0, -0.8, 0.0)),
            (32, DVec3::new(0.0, -1.0, 0.0)),
        ],
        rot: vec![(46, 6.0)],
        ..Mal::default()
    });
    add(
        "premolars",
        &a,
        Config {
            align_scope: AlignScope::Premolars,
            ..Config::default()
        },
        None,
        &[],
        None,
        none(),
    );
    let a = make_arch(&shift(&[
        (41, [1.0, 0.0, 0.0]),
        (31, [1.0, 0.0, 0.0]),
        (32, [0.0, -0.8, 0.0]),
    ]));
    add(
        "midline_balance",
        &a,
        Config {
            midline: Midline::Balance,
            ..Config::default()
        },
        None,
        &[],
        None,
        none(),
    );
    let a = make_arch(&Mal {
        shift: vec![(46, DVec3::new(0.0, -0.6, 0.0))],
        rot: vec![(36, 8.0)],
        ..Mal::default()
    });
    add(
        "molars_free",
        &a,
        Config {
            align_scope: AlignScope::All,
            fix_molars: false,
            fix_third: false,
            align_fixed: false,
            ..Config::default()
        },
        None,
        &[],
        None,
        none(),
    );
    out
}

#[test]
fn the_options_match_the_plugin() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/setup_options.json");
    let fixture: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let cases = &fixture["cases"];
    let num = |v: &serde_json::Value| v.as_f64().unwrap();
    let mut worst = (0.0f64, 0.0f64, 0.0f64);
    let all = scenarios();
    assert_eq!(all.len(), cases.as_object().unwrap().len());
    for (name, r) in &all {
        let want = &cases[*name];
        let keys: Vec<String> = r.targets.keys().map(|f| f.to_string()).collect();
        let want_keys: Vec<String> = want["targets"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        assert_eq!(keys, want_keys, "{name}: moved teeth");
        for (f, d) in &r.targets {
            let m = &want["targets"][f.to_string()];
            let mut rows = [[0.0; 4]; 4];
            for (i, row) in rows.iter_mut().enumerate() {
                for (j, x) in row.iter_mut().enumerate() {
                    *x = num(&m[i][j]);
                }
            }
            let w = DMat4::from_cols_array_2d(&rows).transpose();
            let dt = (d.w_axis - w.w_axis).truncate().length();
            let r3 = DMat3::from_mat4(*d).transpose() * DMat3::from_mat4(w);
            let deg = (((r3.x_axis.x + r3.y_axis.y + r3.z_axis.z) - 1.0) / 2.0)
                .clamp(-1.0, 1.0)
                .acos()
                .to_degrees();
            worst.0 = worst.0.max(dt);
            worst.1 = worst.1.max(deg);
            assert!(dt <= 0.01 && deg <= 0.05, "{name} {f}: {dt} mm, {deg} deg");
        }
        let ipr: Vec<(u8, u8, f64)> = r
            .ipr
            .iter()
            .map(|((a, b), mm)| (a.get(), b.get(), *mm))
            .collect();
        let want_ipr: Vec<(u8, u8, f64)> = want["ipr"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| {
                (
                    x[0].as_u64().unwrap() as u8,
                    x[1].as_u64().unwrap() as u8,
                    num(&x[2]),
                )
            })
            .collect();
        assert_eq!(ipr.len(), want_ipr.len(), "{name}: IPR");
        for (g, w) in ipr.iter().zip(&want_ipr) {
            assert!(
                g.0 == w.0 && g.1 == w.1 && (g.2 - w.2).abs() < 1e-9,
                "{name}: {g:?} {w:?}"
            );
        }
        for (got, key) in [
            (r.expansion, "expansion"),
            (r.expansion_canine, "expansion_canine"),
            (r.advance, "advance"),
            (r.residual, "residual"),
            (r.spacing, "spacing"),
            (r.overlap_max, "overlap_max"),
            (r.midline_shift, "midline_shift"),
        ] {
            assert!(
                (got - num(&want[key])).abs() < 1e-6,
                "{name}: {key} {got} against {}",
                want[key]
            );
        }
        let held: Vec<u64> = r.held.iter().map(|f| u64::from(f.get())).collect();
        let want_held: Vec<u64> = want["held"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_u64().unwrap())
            .collect();
        assert_eq!(held, want_held, "{name}: held");
        let line = want["line"].as_array().unwrap();
        let sampled: Vec<DVec3> = r.line.iter().step_by(8).copied().collect();
        assert_eq!(sampled.len(), line.len(), "{name}: line");
        for (p, q) in sampled.iter().zip(line) {
            let d = (*p - DVec3::new(num(&q[0]), num(&q[1]), num(&q[2]))).length();
            worst.2 = worst.2.max(d);
            assert!(d <= 0.01, "{name}: line off by {d}");
        }
    }
    println!(
        "worst: {:.3e} mm, {:.3e} deg, line {:.3e} mm",
        worst.0, worst.1, worst.2
    );
}
