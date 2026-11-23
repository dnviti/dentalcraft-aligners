//! Tests of the trim: the loop operations, the lines of every mode and the
//! trimmed aligner of the demo arch.

use std::collections::BTreeMap;

use dc_case::Fdi;
use dc_geom::bvh::Bvh;
use dc_geom::sdf::SdfGrid;
use dc_mesh::TriMesh;
use glam::{DVec2, DVec3, Vec3};

use super::line::{self, MarginMode};
use super::survey::{self, circular_smooth, loop_nearest, round_tabs, slope_floor};
use super::*;
use super::{model, warp};
use crate::aligner_bite::occlusal::{Limit, cut_solid};
use crate::aligner_bite::{Grid, Raster};
use crate::anatomy::Arch;
use crate::demo;

#[test]
fn slope_floor_raises_only_and_limits_the_steps() {
    let z = [0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    let f = slope_floor(&z, 0.5);
    assert_eq!(f, vec![2.0, 2.5, 3.0, 2.5, 2.0, 1.5, 1.0, 1.5]);
    assert_eq!(slope_floor(&z, 0.0), z.to_vec());
}

#[test]
fn circular_smooth_is_a_median_then_a_mean() {
    let z = [0.0, 0.0, 9.0, 0.0, 0.0, 0.0];
    // the spike goes with the running median of 5
    assert!(circular_smooth(&z, 5).iter().all(|v| v.abs() < 1e-12));
    let r: Vec<f64> = (0..8).map(f64::from).collect();
    let s = circular_smooth(&r, 5);
    assert!((s[4] - 4.0).abs() < 1e-12);
}

#[test]
fn round_tabs_lifts_a_narrow_v_and_keeps_a_gentle_curve() {
    // a square loop 10 x 10 mm, 40 points, with a narrow V down at one point
    let mut e: Vec<DVec3> = (0..40)
        .map(|k| {
            let t = k as f64 / 10.0;
            let (x, y) = match k / 10 {
                0 => (t * 10.0, 0.0),
                1 => (10.0, (t - 1.0) * 10.0),
                2 => (10.0 - (t - 2.0) * 10.0, 10.0),
                _ => (0.0, 10.0 - (t - 3.0) * 10.0),
            };
            DVec3::new(x, y, 5.0)
        })
        .collect();
    let flat = round_tabs(&e, 1.0, 4);
    assert!(flat.iter().all(|&z| (z - 5.0).abs() < 1e-9));
    e[5].z = 2.0;
    let r = round_tabs(&e, 1.0, 4);
    // points 1 mm apart: the V of one point is narrower than 2 mm and rises
    // to where a disk of 1 mm resting on its sides touches it
    assert!(r[5] > 3.9, "{}", r[5]);
    assert!(r.iter().zip(&e).all(|(a, b)| *a >= b.z - 1e-12));
}

#[test]
fn loop_nearest_tells_inside_from_outside() {
    let e: Vec<DVec3> = (0..64)
        .map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 64.0;
            DVec3::new(5.0 * a.cos(), 5.0 * a.sin(), 1.0 + a.cos())
        })
        .collect();
    let (d, z) = loop_nearest(&e, DVec2::new(1.0, 0.0));
    assert!((d - 4.0).abs() < 0.02 && (z - 2.0).abs() < 0.01, "{d} {z}");
    let (d, _) = loop_nearest(&e, DVec2::new(-7.0, 0.0));
    assert!((d + 2.0).abs() < 0.02, "{d}");
    let lz = survey::loop_z(&e, DVec2::ZERO, &[DVec2::new(0.0, 3.0)]);
    assert!((lz[0] - 1.0).abs() < 0.01, "{lz:?}");
}

#[test]
fn border_floor_keeps_the_line_above_the_border_of_the_scan() {
    let mut c = vec![DVec3::new(0.0, 0.0, 1.0), DVec3::new(10.0, 0.0, 1.0)];
    let border = [DVec3::new(0.5, 0.0, 0.5), DVec3::new(10.0, 0.0, -5.0)];
    line::border_floor(&mut c, &border, 2.0);
    assert_eq!(c[0].z, 1.5);
    assert_eq!(c[1].z, 1.0);
}

#[test]
fn params_are_checked() {
    let p = Params {
        extension_mm: 9.0,
        ..Params::default()
    };
    assert!(p.check().is_err());
    assert!(Params::default().check().is_ok());
    assert_eq!(Params::default().mode, Mode::Smooth);
    assert_eq!(Params::default().extension_mm, 2.0);
}

/// The demo arch as the trim takes it: the closed teeth and the closed
/// gum shell of the demo, the crowns above the gum (what a scan shows),
/// the margin where every tooth enters the gum, the shell of a sheet of
/// `wall` on the closed model.
pub(super) struct DemoArch {
    pub teeth_mesh: TriMesh,
    pub gingiva: TriMesh,
    pub crowns: TriMesh,
    pub teeth: Vec<Tooth>,
    pub shell: SdfGrid,
    pub wall: f64,
    pub voxel: f32,
    /// The closed model the shell is laid on ...
    pub model: TriMesh,
    /// ... with crevices narrower than twice this filled (mm).
    pub blockout: f64,
}

pub(super) fn demo_arch(arch: Arch, wall: f64, voxel: f32) -> DemoArch {
    demo_arch_without(arch, wall, voxel, &[], crate::shell::BLOCKOUT_MM)
}

/// The demo arch without the teeth `missing`: their closed shells taken
/// out of the model, the gum tube left under them (an edentulous space);
/// the shell laid on it with `blockout`.
pub(super) fn demo_arch_without(
    arch: Arch,
    wall: f64,
    voxel: f32,
    missing: &[u16],
    blockout: f64,
) -> DemoArch {
    let d = demo::run(&demo::Params {
        arch,
        ..demo::Params::default()
    });
    let all = d.mesh.vertex_labels.clone().unwrap();
    let mut map = vec![u32::MAX; all.len()];
    let (mut pos, mut labels) = (Vec::new(), Vec::new());
    for (i, l) in all.iter().enumerate() {
        if !missing.contains(l) {
            map[i] = pos.len() as u32;
            pos.push(d.mesh.positions[i]);
            labels.push(*l);
        }
    }
    let tris: Vec<[u32; 3]> = d
        .mesh
        .tris
        .iter()
        .filter(|t| t.iter().all(|&v| map[v as usize] != u32::MAX))
        .map(|t| t.map(|v| map[v as usize]))
        .collect();
    let mut m = TriMesh::new(pos, tris);
    m.vertex_labels = Some(labels.clone());
    let v: Vec<DVec3> = m.positions.iter().map(|p| p.as_dvec3()).collect();
    let parts = model::parts(&v, &m.tris, &labels, &BTreeMap::new(), None);
    assert!(parts.scan_border.is_empty());
    assert!(
        parts
            .teeth
            .iter()
            .all(|t| t.margin.len() >= 20 && t.crown.len() >= 20)
    );
    // the shell: a sheet of `wall` on the closed model (crevices under twice
    // the blockout filled)
    let shell = model::untrimmed_shell(&m, f64::from(voxel), blockout, 0.0, wall).unwrap();
    DemoArch {
        teeth_mesh: parts.teeth_mesh,
        gingiva: parts.gingiva,
        crowns: parts.crowns,
        teeth: parts.teeth,
        shell,
        wall,
        voxel,
        model: m,
        blockout,
    }
}

impl DemoArch {
    pub(super) fn input(&self) -> Input<'_> {
        Input {
            shell: &self.shell,
            teeth_mesh: &self.teeth_mesh,
            gingiva: &self.gingiva,
            crowns: &self.crowns,
            teeth: &self.teeth,
            scan_border: &[],
            clearance_mm: 0.0,
            thickness_mm: self.wall,
            cut: None,
            rebuild: None,
        }
    }

    /// The shell laid on the model with the closed `extra` solids added to
    /// it, as [`Input::rebuild`] wants it.
    pub(super) fn shell_with(&self, extra: &[TriMesh]) -> Result<SdfGrid, String> {
        let mut pos = self.model.positions.clone();
        let mut tris = self.model.tris.clone();
        for e in extra {
            let off = pos.len() as u32;
            pos.extend_from_slice(&e.positions);
            tris.extend(e.tris.iter().map(|t| t.map(|v| v + off)));
        }
        model::untrimmed_shell(
            &TriMesh::new(pos, tris),
            f64::from(self.voxel),
            self.blockout,
            0.0,
            self.wall,
        )
        .map_err(|e| e.to_string())
    }
}

fn check_solid(out: &Output) {
    let report = out.mesh.validate();
    assert!(report.is_watertight() && report.is_manifold(), "{report:?}");
    assert_eq!(report.components, 1);
}

#[test]
fn straight_line_on_the_demo_arch() {
    let a = demo_arch(Arch::Lower, 0.6, 0.25);
    let p = Params {
        mode: Mode::Straight,
        ..Params::default()
    };
    let t0 = std::time::Instant::now();
    let out = run(&a.input(), &p).unwrap();
    eprintln!(
        "straight: {} tris, genus {}, dropped {}, fill {:?}, plugs {}, slots {}, {:.1?}",
        out.mesh.face_count(),
        out.genus,
        out.dropped_pieces,
        out.fill,
        out.plugs,
        out.slots.len(),
        t0.elapsed()
    );
    check_solid(&out);
    assert_eq!(out.mode, Mode::Straight);
    // the plugin's rule: every side of a tooth at its gingival zenith less the extension
    for (t, l) in a.teeth.iter().zip(&out.lines) {
        assert_eq!(Some(t.fdi), l.fdi);
        assert_eq!(l.points.len(), 2, "{}", t.fdi);
        let step = (t.margin.len() / 60).max(1);
        let m: Vec<DVec3> = t.margin.iter().step_by(step).copied().collect();
        for (want_buccal, q) in [(true, l.points[0]), (false, l.points[1])] {
            let zen = m
                .iter()
                .filter(|v| ((**v - t.origin).dot(t.e_bl) > 0.0) == want_buccal)
                .map(|v| v.z)
                .fold(f64::INFINITY, f64::min);
            assert!((q.z - (zen - p.extension_mm)).abs() < 1e-9, "{}", t.fdi);
            // the trim surface passes within 0.1 mm of the line there
            let s = out.surface_at(q.x, q.y);
            assert!((s - q.z).abs() < 0.1, "{} {s} {}", t.fdi, q.z);
        }
    }
    // nothing of the aligner is below the trim surface (the rim is rounded: up to a voxel)
    let tol = f64::from(a.voxel) + 0.05;
    let low = out
        .mesh
        .positions
        .iter()
        .map(|v| v.z as f64 - out.surface_at(v.x as f64, v.y as f64))
        .fold(f64::INFINITY, f64::min);
    assert!(low > -tol, "{low}");
    // and the shell is kept above the trim surface: the points of the
    // untrimmed shell near the line, 0.5 mm above the surface and 0.5 mm
    // within the reach from the crowns (extension + 1.5 mm), are still on
    // the aligner (the cut is where the line is, not higher; the rounded rim
    // takes off a few thin webs between the teeth)
    let untrimmed = a.shell.to_mesh().unwrap();
    let trimmed = Bvh::build(&out.mesh).unwrap();
    let crowns = Bvh::build(&a.crowns).unwrap();
    let reach = (p.extension_mm + cutter::NEAR_TEETH_EXTRA) as f32;
    let (mut kept, mut all) = (0usize, 0usize);
    for l in &out.lines {
        let q = l.points[0];
        for v in untrimmed.positions.iter().filter(|v| {
            (v.as_dvec3().truncate() - q.truncate()).length() < 3.0
                && f64::from(v.z) > out.surface_at(f64::from(v.x), f64::from(v.y)) + 0.5
        }) {
            if crowns
                .closest_point(*v, 50.0)
                .is_some_and(|c| c.distance < reach - 0.5)
            {
                all += 1;
                kept += usize::from(
                    trimmed
                        .closest_point(*v, 1.0)
                        .is_some_and(|h| h.distance < a.voxel),
                );
            }
        }
    }
    eprintln!("kept {kept} of {all}");
    assert!(
        all > 1000 && kept as f64 > 0.99 * all as f64,
        "kept {kept} of {all}"
    );
}

#[test]
fn equator_line_on_the_demo_arch() {
    let a = demo_arch(Arch::Upper, 0.6, 0.25);
    let p = Params {
        mode: Mode::Equator,
        insertion_axis: warp::InsertionAxis::Vertical,
        ..Params::default()
    };
    let t0 = std::time::Instant::now();
    let out = run(&a.input(), &p).unwrap();
    eprintln!(
        "equator: {} tris, genus {}, dropped {}, {:.1?}",
        out.mesh.face_count(),
        out.genus,
        out.dropped_pieces,
        t0.elapsed()
    );
    check_solid(&out);
    let by: BTreeMap<Fdi, &Tooth> = a.teeth.iter().map(|t| (t.fdi, t)).collect();
    for l in &out.lines {
        let fdi = l.fdi.unwrap();
        let t = by[&fdi];
        let eq = line::tooth_equator(t, line::EQUATOR_AZIMUTHS).unwrap();
        let top = eq.top;
        let zm = eq.margin.clone().unwrap();
        assert_eq!(l.points.len(), line::EQUATOR_AZIMUTHS);
        for (k, q) in l.points.iter().enumerate() {
            // the plugin's limits: under the top less 1 mm, over the margin plus 0.1 mm
            let floor = (zm[k] + line::EQUATOR_GUM_GAP_MM).min(top - line::EQUATOR_GUM_GAP_MM);
            assert!(q.z >= floor - 1e-9, "{fdi} {k}");
            assert!(
                q.z <= (top - line::EQUATOR_TOP_MM).max(floor) + 1e-9,
                "{fdi} {k}"
            );
            // offset 0: the line is the equator where no tab is rounded
            assert!(q.z >= eq.points[k].z - 1e-9);
        }
        // the equator is the height of contour: no crown point beyond the
        // loop in plan is far below it
        let c = eq.centre;
        let mean_eq = eq.points.iter().map(|q| q.z).sum::<f64>() / eq.points.len() as f64;
        assert!(
            mean_eq < top - line::EQUATOR_TOP_MM + 1e-9
                && mean_eq > zm.iter().copied().fold(f64::INFINITY, f64::min)
        );
        let _ = c;
    }
    // every crown keeps its cap: the aligner covers the top of every crown
    let bvh = Bvh::build(&out.mesh).unwrap();
    for t in &a.teeth {
        let top = t
            .crown
            .iter()
            .copied()
            .reduce(|a, b| if b.z > a.z { b } else { a })
            .unwrap();
        let h = bvh.ray_cast((top + DVec3::Z * 10.0).as_vec3(), Vec3::NEG_Z, 20.0);
        assert!(h.is_some(), "{} uncovered", t.fdi);
    }
}

#[test]
fn equator_along_the_insertion_path_of_every_tooth() {
    let a = demo_arch(Arch::Lower, 0.6, 0.25);
    let p = Params {
        mode: Mode::Equator,
        ..Params::default()
    };
    assert_eq!(p.insertion_axis, warp::InsertionAxis::PerTooth);
    let out = run(&a.input(), &p).unwrap();
    eprintln!(
        "per tooth: genus {}, dropped {}",
        out.genus, out.dropped_pieces
    );
    check_solid(&out);
    // the warp takes the points back where they were
    if let Some(w) = &out.warp {
        let max_tilt = w
            .shears
            .iter()
            .map(|s| s.length().atan().to_degrees())
            .fold(0.0, f64::max);
        eprintln!("largest tilt {max_tilt:.1} deg");
        assert!(max_tilt <= 30.0 + 1e-6);
        for t in &a.teeth {
            for q in t.crown.iter().step_by(37) {
                assert!((w.from_warp(w.to_warp(*q)) - *q).length() < 1e-6);
            }
        }
    }
    // every crown keeps its cap
    let bvh = Bvh::build(&out.mesh).unwrap();
    for t in &a.teeth {
        let top = t
            .crown
            .iter()
            .copied()
            .reduce(|a, b| if b.z > a.z { b } else { a })
            .unwrap();
        assert!(
            bvh.ray_cast((top + DVec3::Z * 10.0).as_vec3(), Vec3::NEG_Z, 20.0)
                .is_some(),
            "{}",
            t.fdi
        );
    }
}

#[test]
fn smooth_border_on_the_demo_arch() {
    let mut a = demo_arch(Arch::Lower, 0.6, 0.25);
    let p = Params::default();
    let out = run(&a.input(), &p).unwrap();
    eprintln!(
        "smooth: {} tris, genus {}, dropped {}",
        out.mesh.face_count(),
        out.genus,
        out.dropped_pieces
    );
    assert_eq!(out.mode, Mode::Smooth);
    assert!(!out.fallback);
    check_solid(&out);
    assert_eq!(out.lines.len(), 1);
    assert_eq!(out.lines[0].fdi, None);
    // the trim surface passes through the border points, which sit
    // margin_gap_mm above the margin within the scallops
    let zmin = a
        .teeth
        .iter()
        .flat_map(|t| t.margin.iter().map(|q| q.z))
        .fold(f64::INFINITY, f64::min);
    let zmax = a
        .teeth
        .iter()
        .flat_map(|t| t.margin.iter().map(|q| q.z))
        .fold(f64::NEG_INFINITY, f64::max);
    // (but for the last rows round the distal faces, next to the high
    // points behind them)
    let pts = &out.lines[0].points;
    let mut on = 0;
    for q in pts {
        assert!(
            q.z > zmin + p.margin_gap_mm - p.scallop_mm
                && q.z < zmax + p.margin_gap_mm + p.scallop_mm
        );
        on += usize::from((out.surface_at(q.x, q.y) - q.z).abs() < 0.1);
    }
    assert!(on as f64 > 0.9 * pts.len() as f64, "{on} of {}", pts.len());
    // one covered tooth only: the scalloped border instead, as the plugin
    for t in &mut a.teeth {
        t.covered = t.fdi.get() == 36;
    }
    let out = run(&a.input(), &p).unwrap();
    assert_eq!(out.mode, Mode::Scalloped);
    assert!(out.fallback);
    check_solid(&out);
    let m = line::margin_points(&a.teeth, MarginMode::Scalloped, p.extension_mm);
    assert_eq!(out.lines.len(), m.len());
}

/// The demo arch with a papilla between every two neighbours (its gum is
/// flat between the teeth): every margin point within 2 mm (plan) of
/// another crown raised by up to `rise` mm, the most next to that crown.
fn raise_papillae(a: &mut DemoArch, rise: f64) {
    let crowns: Vec<(Fdi, Vec<DVec2>)> = a
        .teeth
        .iter()
        .map(|t| (t.fdi, t.crown.iter().map(|p| p.truncate()).collect()))
        .collect();
    for t in &mut a.teeth {
        for q in &mut t.margin {
            let d = crowns
                .iter()
                .filter(|(f, _)| *f != t.fdi)
                .flat_map(|(_, c)| c.iter().map(|p| p.distance(q.truncate())))
                .fold(f64::INFINITY, f64::min);
            let w = (1.0 - d / 2.0).max(0.0);
            q.z += rise * w * w;
        }
    }
}

/// At every margin point within 1 mm (plan) of another crown (the top of a
/// papilla of [`raise_papillae`]): how far the edge of the trimmed aligner
/// (its lowest point within 0.35 mm, plan) stands above the height it must
/// reach, `relief` above the point or [`smooth::PAPILLA_NECK_MM`] under the
/// contact of the two crowns when that is lower. The smallest, and how many
/// points were measured.
fn papilla_slack(a: &DemoArch, out: &Output, relief: f64) -> (f64, usize) {
    let flat: Vec<Vec3> = out
        .mesh
        .positions
        .iter()
        .map(|p| Vec3::new(p.x, p.y, 0.0))
        .collect();
    let index = dc_geom::knn::KnnIndex::build(&flat).unwrap();
    let contact = |f: Fdi, g: Fdi| -> Option<DVec3> {
        let (ca, cb) = (
            &a.teeth.iter().find(|t| t.fdi == f)?.crown,
            &a.teeth.iter().find(|t| t.fdi == g)?.crown,
        );
        let mut best = (f64::INFINITY, DVec3::ZERO);
        for p in ca {
            for q in cb {
                if p.distance(*q) < best.0 {
                    best = (p.distance(*q), (*p + *q) * 0.5);
                }
            }
        }
        (best.0 <= smooth::PAPILLA_CONTACT_MM).then_some(best.1)
    };
    let (mut slack, mut n) = (f64::INFINITY, 0);
    for t in a.teeth.iter().filter(|t| t.covered) {
        for q in &t.margin {
            // the crown nearest to the point, of another covered tooth
            let Some((g, d)) = a
                .teeth
                .iter()
                .filter(|u| u.fdi != t.fdi && u.covered)
                .map(|u| {
                    let d = u
                        .crown
                        .iter()
                        .map(|p| p.truncate().distance(q.truncate()))
                        .fold(f64::INFINITY, f64::min);
                    (u.fdi, d)
                })
                .min_by(|x, y| x.1.total_cmp(&y.1))
            else {
                continue;
            };
            if d > 1.0 {
                continue;
            }
            let edge = index
                .within_radius(Vec3::new(q.x as f32, q.y as f32, 0.0), 0.35)
                .iter()
                .map(|nb| f64::from(out.mesh.positions[nb.index as usize].z))
                .fold(f64::INFINITY, f64::min);
            if !edge.is_finite() {
                continue;
            }
            let cap = contact(t.fdi, g).map_or(f64::INFINITY, |c| c.z - smooth::PAPILLA_NECK_MM);
            slack = slack.min(edge - (q.z + relief).min(cap));
            n += 1;
        }
    }
    (slack, n)
}

#[test]
fn the_smooth_border_goes_round_every_papilla() {
    // the user's rule: the edge of the aligner never comes down onto a
    // papilla between two teeth; it stays papilla_relief_mm above the margin
    // (or 0.5 mm under the contact of the two crowns), in a rounded notch
    let mut a = demo_arch(Arch::Lower, 0.6, 0.25);
    raise_papillae(&mut a, 3.0);
    let tol = f64::from(a.voxel) + 0.05;
    let run_with = |relief: f64| {
        let p = Params {
            papilla_relief_mm: relief,
            ..Params::default()
        };
        run(&a.input(), &p).unwrap()
    };
    // the plugin's border (no relief) comes down onto the papillae
    let plugin = run_with(0.0);
    let (slack, n) = papilla_slack(&a, &plugin, smooth::PAPILLA_RELIEF_MM);
    assert!(n >= 20, "{n} papilla points measured");
    assert!(
        slack < -tol,
        "the plugin's border clears the papillae: {slack}"
    );
    // the default goes round them, the aligner a closed solid as before
    let p = Params::default();
    assert_eq!(p.papilla_relief_mm, smooth::PAPILLA_RELIEF_MM);
    let out = run(&a.input(), &p).unwrap();
    let (slack, n) = papilla_slack(&a, &out, p.papilla_relief_mm);
    eprintln!(
        "papillae: {n} points, slack {slack:.3} mm (plugin {:.3}); genus {} plugs {} slots {} \
         (plugin {} {} {})",
        papilla_slack(&a, &plugin, p.papilla_relief_mm).0,
        out.genus,
        out.plugs,
        out.slots.len(),
        plugin.genus,
        plugin.plugs,
        plugin.slots.len()
    );
    assert!(n >= 20, "{n} papilla points measured");
    assert!(slack >= -tol, "the edge comes down onto a papilla: {slack}");
    assert_eq!(out.mode, Mode::Smooth);
    assert_eq!(out.papilla_relief_mm, p.papilla_relief_mm);
    check_solid(&out);
    assert_eq!(out.genus, 0);
    assert!(out.plugs <= plugin.plugs && out.slots.len() <= plugin.slots.len());
    // the wall keeps its thickness, as the print check measures it (the
    // concave corners of the rim within its limit are not walls)
    let options = crate::export::printcheck::CheckOptions {
        min_thickness_mm: Some(0.5 * a.wall),
        build_plate_mm: None,
        ..crate::export::printcheck::CheckOptions::default()
    };
    let thin = |o: &Output| {
        crate::export::printcheck::check(&o.mesh, &options)
            .min_thickness_mm
            .unwrap()
    };
    let (t, t0) = (thin(&out), thin(&plugin));
    eprintln!("thinnest wall {t:.3} mm (plugin {t0:.3})");
    assert!(t >= t0.min(0.5 * a.wall) - 0.05, "{t} < {t0}");
}

#[test]
fn fills_only_for_a_thin_wall() {
    // the 0.5 mm wall rounded by 0.15: 0.2 mm left, the fills are tried
    // (`aligner.shell_fills`): 0.5, 0.5 also beyond the outer side, 0.75
    let f = |rho, out| Fill { rho, out };
    assert_eq!(
        shell_fills(0.5, 0.15),
        vec![
            f(SHELL_FILL_MM, 0.0),
            f(SHELL_FILL_MM, SHELL_FILL_GROW_MM),
            f(SHELL_FILL_WIDE_MM, SHELL_FILL_GROW_MM)
        ]
    );
    // the 1.0 mm wall rounded by 0.3: 0.4 mm left, no fill
    assert!(shell_fills(1.0, 0.3).is_empty());
    assert!(shell_fills(0.5, 0.0).is_empty());
    assert_eq!(rim_round(0.6, None), 0.18);
    assert_eq!(rim_round(1.5, None), 0.3);
}

/// The wall of a ball between the radii `radius` and `radius + wall` over the
/// plane `z = -1`, and the solid above an ellipsoid as wide as its outer side
/// and `lower` less high, on the grid of the occlusal shaping tilted by `tilt`
/// (rad) and with some nodes without a top (`gaps`, the share of them): the cut
/// fades out at a grazing angle towards the equator and stops on the steps of
/// its cells, as the occlusal shaping does on the sides of a crown.
fn grazing_cut(
    voxel: f32,
    radius: f32,
    wall: f32,
    lower: f64,
    tilt: f32,
    gaps: f64,
) -> [SdfGrid; 2] {
    let outer = radius + wall;
    let (nu, nv) = (240usize, 120usize);
    let mut positions = vec![Vec3::new(0.0, 0.0, radius), Vec3::new(0.0, 0.0, -radius)];
    for j in 1..nv {
        let phi = std::f32::consts::PI * j as f32 / nv as f32;
        for i in 0..nu {
            let th = std::f32::consts::TAU * i as f32 / nu as f32;
            positions
                .push(radius * Vec3::new(phi.sin() * th.cos(), phi.sin() * th.sin(), phi.cos()));
        }
    }
    let at = |i: usize, j: usize| (2 + (j - 1) * nu + i % nu) as u32;
    let mut tris = Vec::new();
    for i in 0..nu {
        tris.push([0, at(i, 1), at(i + 1, 1)]);
        tris.push([1, at(i + 1, nv - 1), at(i, nv - 1)]);
        for j in 1..nv - 1 {
            tris.push([at(i, j), at(i, j + 1), at(i + 1, j + 1)]);
            tris.push([at(i, j), at(i + 1, j + 1), at(i + 1, j)]);
        }
    }
    let params = SdfParams {
        voxel_size: voxel,
        band: wall + 4.0 * voxel,
    };
    let ball = SdfGrid::from_mesh(&TriMesh::new(positions, tris), &params).unwrap();
    let mut shell = ball.shell(0.0, wall).unwrap();
    let side = f64::from(outer) + 2.0;
    let plan = PlanGrid {
        lo: DVec2::splat(-side),
        hi: DVec2::splat(side),
        nx: 2,
        ny: 2,
    };
    let above = grid::slab_solid(&plan, &[-1.0; 4], side);
    shell
        .intersect(&SdfGrid::from_mesh(&above, &params).unwrap())
        .unwrap();
    shell.redistance().unwrap();
    let (a, c) = (f64::from(outer), f64::from(outer) - lower);
    let cell = crate::aligner_bite::occlusal::CELL;
    let g = Grid::new([-a - 1.0, -a - 1.0], [a + 1.0, a + 1.0], cell);
    let mut top = Raster::filled(g.n, f64::INFINITY);
    for i in 0..g.n[0] {
        for j in 0..g.n[1] {
            let (x, y) = (g.lo[0] + i as f64 * cell, g.lo[1] + j as f64 * cell);
            let r2 = (x * x + y * y) / (a * a);
            if r2 <= 1.0 && ((i * 7 + j * 13) as f64 * 1.37).sin().abs() >= gaps {
                top.data[i * g.n[1] + j] = (1.0 - r2).sqrt() * c;
            }
        }
    }
    let limit = Limit {
        top,
        grid: g,
        min_thickness: 0.5,
    };
    let mut solid = cut_solid(&limit, 5.0).unwrap();
    let turn = glam::Quat::from_rotation_y(tilt) * glam::Quat::from_rotation_x(0.6 * tilt);
    for p in &mut solid.positions {
        *p = turn * *p;
    }
    let cutter = SdfGrid::from_mesh(&solid, &params).unwrap();
    [shell, cutter]
}

/// Pieces and handles of the zero level of `g`.
fn pieces_and_genus(g: &SdfGrid) -> (usize, i64) {
    let m = g.to_mesh().unwrap();
    let n = m.connected_components().len() as i64;
    let chi =
        m.vertex_count() as i64 - m.edge_adjacency().edge_count() as i64 + m.face_count() as i64;
    (n as usize, n - chi / 2)
}

#[test]
fn a_grazing_cut_leaves_no_specks_and_no_handles() {
    let [shell, cutter] = grazing_cut(0.1, 5.0, 1.0, 0.25, 0.07, 0.07);
    assert_eq!(pieces_and_genus(&shell), (1, 0));
    // the plain difference leaves the skin as specks and a tunnel in it
    let mut plain = shell.clone();
    plain.subtract(&cutter).unwrap();
    let (pieces, handles) = pieces_and_genus(&plain);
    assert!(
        pieces > 1 && handles > 0,
        "{pieces} pieces, {handles} handles"
    );
    let mut cut = shell.clone();
    cut_away(&mut cut, &cutter).unwrap();
    assert_eq!(pieces_and_genus(&cut), (1, 0));
    // and takes the skin and half a voxel, nothing else
    let (a, _) = largest_piece(plain.to_mesh().unwrap());
    let (b, _) = largest_piece(cut.to_mesh().unwrap());
    let d = crate::shell::deviation(&a, &b).unwrap();
    assert!(d <= 0.15, "{d}");
}

#[test]
fn the_outer_side_of_the_wall_grows_and_the_fitting_surface_stays() {
    // the untrimmed shell of a 6 mm cube: clearance 0, wall 0.5
    let cube = crate::export::printcheck::tests::cube(Vec3::ZERO, 6.0);
    let params = dc_geom::sdf::SdfParams {
        voxel_size: 0.1,
        band: 1.2,
    };
    let wall = SdfGrid::from_mesh(&cube, &params)
        .unwrap()
        .shell(0.0, 0.5)
        .unwrap();
    let grown = outer_grown(&wall, SHELL_FILL_GROW_MM).unwrap();
    // along the normal of the top face, from its centre: (distance from the face, in the wall, in the grown wall)
    for (d, in_wall, in_grown) in [
        (-0.3, false, false), // the cavity: the fitting surface stays
        (-0.05, false, false),
        (0.25, true, true),
        (0.58, false, true), // beyond the outer side, within the growth
        (0.8, false, false),
    ] {
        let p = Vec3::new(3.0, 3.0, 6.0 + d);
        assert_eq!(
            wall.sample(p) < 0.0,
            in_wall,
            "wall at {d}: {}",
            wall.sample(p)
        );
        assert_eq!(
            grown.sample(p) < 0.0,
            in_grown,
            "grown at {d}: {}",
            grown.sample(p)
        );
    }
}

/// The highest crown point of every covered tooth is under the aligner (a
/// ray down from above it meets the aligner).
fn uncapped(out: &Output, teeth: &[Tooth]) -> Vec<Fdi> {
    let bvh = Bvh::build(&out.mesh).unwrap();
    teeth
        .iter()
        .filter(|t| t.covered)
        .filter(|t| {
            let top = t
                .crown
                .iter()
                .copied()
                .reduce(|a, b| if b.z > a.z { b } else { a })
                .unwrap();
            bvh.ray_cast((top + DVec3::Z * 10.0).as_vec3(), Vec3::NEG_Z, 20.0)
                .is_none()
        })
        .map(|t| t.fdi)
        .collect()
}

#[test]
fn a_missing_central_incisor_is_bridged_and_the_aligner_stays_whole() {
    // the demo upper arch without 21: the space of a central incisor between
    // 11 and 22, wider than the reach of the aligner from the two crowns; the
    // shell laid with the blockout of the border, as the export lays it
    let (f11, f22) = (Fdi::new(11).unwrap(), Fdi::new(22).unwrap());
    for mode in [Mode::Smooth, Mode::Equator] {
        let blockout = if mode == Mode::Smooth {
            crate::shell::SMOOTH_BLOCKOUT_MM
        } else {
            crate::shell::BLOCKOUT_MM
        };
        let a = demo_arch_without(Arch::Upper, 1.0, 0.25, &[21], blockout);
        assert!(a.teeth.iter().all(|t| t.fdi.get() != 21));
        let shell_with = |extra: &[TriMesh]| a.shell_with(extra);
        let p = Params {
            mode,
            insertion_axis: warp::InsertionAxis::Vertical,
            ..Params::default()
        };
        // without a way to build the shell again, the aligner that comes
        // apart over the space is refused, never cut down to its largest piece
        match run(&a.input(), &p) {
            Err(TrimError::Split(lost)) => {
                assert!(!lost.is_empty(), "{mode:?}");
                assert!(lost.iter().all(|f| a.teeth.iter().any(|t| t.fdi == *f)));
            }
            Err(e) => panic!("{mode:?}: {e}"),
            Ok(out) => panic!(
                "{mode:?}: kept {} pieces dropped, uncapped {:?}",
                out.dropped_pieces,
                uncapped(&out, &a.teeth)
            ),
        }
        // with it, a bar joins 11 and 22 over the space and the aligner is one
        // piece over the whole arch
        let input = Input {
            rebuild: Some(Rebuild(&shell_with)),
            ..a.input()
        };
        let t0 = std::time::Instant::now();
        let out = run(&input, &p).unwrap();
        eprintln!(
            "{mode:?}: {} tris, bars {:?}, weak {:?}, dropped {}, genus {}, {:.1?}",
            out.mesh.face_count(),
            out.bars.iter().map(|b| b.teeth).collect::<Vec<_>>(),
            out.weak,
            out.dropped_pieces,
            out.genus,
            t0.elapsed()
        );
        check_solid(&out);
        // no hole through the wall
        assert_eq!(out.genus, 0, "{mode:?}");
        assert!(
            out.bars.iter().any(|b| b.teeth == (f11, f22)),
            "{mode:?}: {:?}",
            out.bars.iter().map(|b| b.teeth).collect::<Vec<_>>()
        );
        assert!(
            out.weak.iter().all(|w| !w.split),
            "{mode:?}: {:?}",
            out.weak
        );
        assert_eq!(uncapped(&out, &a.teeth), Vec::<Fdi>::new(), "{mode:?}");
        // the bar lies over the space: the aligner covers the middle of it
        let bar = out.bars.iter().find(|b| b.teeth == (f11, f22)).unwrap();
        let rows = bar.path.len() / bars::PATH_COLUMNS;
        let mid = bar.path[(rows / 2) * bars::PATH_COLUMNS + bars::PATH_COLUMNS / 2];
        let bvh = Bvh::build(&out.mesh).unwrap();
        assert!(
            bvh.ray_cast((mid + DVec3::Z * 10.0).as_vec3(), Vec3::NEG_Z, 20.0)
                .is_some(),
            "{mode:?}: nothing over the space"
        );
    }
}

#[test]
fn the_reach_over_a_bar_is_the_reach_of_the_crowns_and_its_top() {
    // a strip over the space between two incisors, 1 mm above them
    let a = demo_arch(Arch::Lower, 0.6, 0.25);
    let top = |f: u8| {
        let t = a.teeth.iter().find(|t| t.fdi.get() == f).unwrap();
        t.crown
            .iter()
            .copied()
            .reduce(|p, q| if q.z > p.z { q } else { p })
            .unwrap()
    };
    let (p, q) = (top(31), top(42));
    let across = (q - p).cross(DVec3::Z).normalize();
    let rows = 12;
    let path: Vec<DVec3> = (0..rows)
        .flat_map(|i| {
            let c = p + (q - p) * (i as f64 / (rows - 1) as f64) + DVec3::Z;
            (0..bars::PATH_COLUMNS).map(move |j| c + across * (j as f64 - 2.0) * 0.8)
        })
        .collect();
    let bar = bars::Bar {
        teeth: (Fdi::new(31).unwrap(), Fdi::new(42).unwrap()),
        p0: p.truncate(),
        p1: q.truncate(),
        z0: p.z,
        z1: q.z,
        solid: TriMesh::new(Vec::new(), Vec::new()),
        path,
    };
    let r = 0.6 + smooth::SMOOTH_NEAR_MM;
    let both = {
        let s = strips(std::slice::from_ref(&bar));
        let mut pos = a.crowns.positions.clone();
        let mut tris = a.crowns.tris.clone();
        let off = pos.len() as u32;
        pos.extend(s.positions);
        tris.extend(s.tris.iter().map(|t| t.map(|v| v + off)));
        near_grid(&TriMesh::new(pos, tris), r, &a.shell).unwrap()
    };
    let crowns = near_grid(&a.crowns, r, &a.shell).unwrap();
    let united = near_with_strips(crowns.clone(), &[bar], r, &a.shell).unwrap();
    let (mut same, mut all, mut grown) = (0usize, 0usize, 0usize);
    both.for_each_voxel(|ijk, _| {
        let x = both.voxel_position(ijk);
        all += 1;
        same += usize::from(both.contains(x) == united.contains(x));
        grown += usize::from(united.contains(x) && !crowns.contains(x));
    });
    // the same region, the strip taken in
    assert!(grown > 0);
    assert!(same as f64 >= 0.9999 * all as f64, "{same} of {all}");
}

/// The margin of the demo arch in arch order with the distal half of the
/// last tooth of each side raised to `below` under the top of its crown (a
/// flange of the scan at the height of the crown behind it: the label of the
/// tooth stops on its distal ridge), the crowns, the covered teeth and the
/// end teeth with the centre of their crown and the distal direction.
#[allow(clippy::type_complexity)]
fn demo_flanged(
    a: &DemoArch,
    below: f64,
) -> (
    Vec<(Fdi, Vec<DVec3>)>,
    BTreeMap<Fdi, Vec<DVec3>>,
    BTreeSet<Fdi>,
    Vec<(Fdi, DVec2, DVec2)>,
) {
    let order = smooth_order(&a.input());
    let crowns: BTreeMap<Fdi, Vec<DVec3>> =
        a.teeth.iter().map(|t| (t.fdi, t.crown.clone())).collect();
    let covered: BTreeSet<Fdi> = a.teeth.iter().map(|t| t.fdi).collect();
    let centre =
        |f: Fdi| crowns[&f].iter().map(|p| p.truncate()).sum::<DVec2>() / crowns[&f].len() as f64;
    let n = order.len();
    let ends: Vec<(Fdi, DVec2, DVec2)> = [(order[0], order[1]), (order[n - 1], order[n - 2])]
        .into_iter()
        .map(|(f, g)| (f, centre(f), (centre(f) - centre(g)).normalize()))
        .collect();
    let marg = order
        .iter()
        .map(|&f| {
            let mut m = a.teeth.iter().find(|t| t.fdi == f).unwrap().margin.clone();
            if let Some((_, c, t)) = ends.iter().find(|e| e.0 == f) {
                let top = crowns[&f]
                    .iter()
                    .map(|p| p.z)
                    .fold(f64::NEG_INFINITY, f64::max);
                for p in &mut m {
                    if (p.truncate() - *c).dot(*t) > 0.0 {
                        p.z = top - below;
                    }
                }
            }
            (f, m)
        })
        .collect();
    (marg, crowns, covered, ends)
}

#[test]
fn the_border_rounds_the_last_tooth_at_the_gum_not_on_a_flange() {
    // HQ2: behind 16 the scan stands at the height of the crown, the margin of
    // its distal half on the distal ridge; the border taken from it (+ the gap)
    // lay over the crown and cut a notch into the end of the aligner
    let a = demo_arch(Arch::Lower, 0.6, 0.25);
    let p = Params::default();
    let (marg, crowns, covered, ends) = demo_flanged(&a, 0.5);
    let (border, high) = smooth::smooth_border(
        &marg,
        &crowns,
        &covered,
        0.6,
        p.margin_gap_mm,
        p.scallop_mm,
        p.papilla_relief_mm,
    )
    .unwrap();
    for (f, c, t) in &ends {
        let crown = &crowns[f];
        let top = crown.iter().map(|q| q.z).fold(f64::NEG_INFINITY, f64::max);
        let reach = crown
            .iter()
            .map(|q| (q.truncate() - *c).dot(*t))
            .fold(0.0, f64::max);
        // the border round the distal half stays at least 2 mm under the top
        // of the crown: the distal face covered, no notch
        let cap: Vec<f64> = border
            .iter()
            .filter(|q| {
                let s = (q.truncate() - *c).dot(*t);
                s > 0.0 && s < reach + 2.0 && (q.truncate() - *c).length() < 12.0
            })
            .map(|q| q.z)
            .collect();
        assert!(!cap.is_empty(), "{f}");
        let z = cap.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        assert!(z < top - 2.0, "{f}: border at {z:.2}, crown top {top:.2}");
        // and nothing is kept behind it: rows of high points from just past
        // the shell to 8 mm on, past the end of the line of the arch too
        let behind: Vec<f64> = high
            .iter()
            .map(|q| (q.truncate() - *c).dot(*t) - reach)
            .filter(|&s| s > 0.0)
            .collect();
        let far = behind.iter().copied().fold(0.0, f64::max);
        assert!(
            far > 7.5,
            "{f}: high points only {far:.2} mm behind the crown"
        );
    }
}

#[test]
fn past_the_last_tooth_the_aligner_reaches_only_near_its_crown() {
    let a = demo_arch(Arch::Lower, 0.6, 0.25);
    let input = a.input();
    let order = smooth_order(&input);
    let (r, e) = (
        0.6 + smooth::SMOOTH_NEAR_MM,
        0.6 + smooth::SMOOTH_END_NEAR_MM,
    );
    let plain = near_grid(&a.crowns, r, &a.shell).unwrap();
    let ends = smooth_near(&input, &order, r, e, &a.shell).unwrap();
    let (first, last) = (order[0], order[order.len() - 1]);
    let (mut pts, mut at_end) = (Vec::new(), Vec::new());
    for t in &a.teeth {
        pts.extend(t.crown.iter().map(|p| p.as_vec3()));
        at_end.extend(std::iter::repeat_n(
            t.fdi == first || t.fdi == last,
            t.crown.len(),
        ));
    }
    let index = dc_geom::knn::KnnIndex::build(&pts).unwrap();
    let slop = 2.0 * f64::from(a.voxel);
    let (mut dropped, mut extra, mut wrong) = (0usize, 0usize, 0usize);
    plain.for_each_voxel(|ijk, _| {
        let x = plain.voxel_position(ijk);
        let (inp, ine) = (plain.contains(x), ends.contains(x));
        extra += usize::from(ine && !inp);
        if inp && !ine {
            dropped += 1;
            // only near the end teeth, farther than `e` from their crowns
            let n = index.nearest(x).unwrap();
            wrong += usize::from(!at_end[n.index as usize] || f64::from(n.distance) < e - slop);
        }
    });
    assert!(dropped > 0);
    assert_eq!(extra, 0);
    assert_eq!(wrong, 0, "{wrong} of {dropped}");
}
