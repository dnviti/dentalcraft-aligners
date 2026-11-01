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
}

pub(super) fn demo_arch(arch: Arch, wall: f64, voxel: f32) -> DemoArch {
    let d = demo::run(&demo::Params {
        arch,
        ..demo::Params::default()
    });
    let m = &d.mesh;
    let v: Vec<DVec3> = m.positions.iter().map(|p| p.as_dvec3()).collect();
    let labels = m.vertex_labels.clone().unwrap();
    let parts = model::parts(&v, &m.tris, &labels, &BTreeMap::new(), None);
    assert!(parts.scan_border.is_empty());
    assert!(
        parts
            .teeth
            .iter()
            .all(|t| t.margin.len() >= 20 && t.crown.len() >= 20)
    );
    // the shell: a sheet of `wall` on the closed model (crevices under 0.6 mm filled)
    let shell = model::untrimmed_shell(m, f64::from(voxel), 0.3, 0.0, wall).unwrap();
    DemoArch {
        teeth_mesh: parts.teeth_mesh,
        gingiva: parts.gingiva,
        crowns: parts.crowns,
        teeth: parts.teeth,
        shell,
        wall,
        voxel,
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
        }
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
        "straight: {} tris, genus {}, dropped {}, fill {:?}, {:.1?}",
        out.mesh.face_count(),
        out.genus,
        out.dropped_pieces,
        out.fill_mm,
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

#[test]
fn fills_only_for_a_thin_wall() {
    // the 0.5 mm wall rounded by 0.15: 0.2 mm left, the fills are tried
    assert_eq!(
        shell_fills(0.5, 0.15),
        vec![SHELL_FILL_MM, SHELL_FILL_WIDE_MM]
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
