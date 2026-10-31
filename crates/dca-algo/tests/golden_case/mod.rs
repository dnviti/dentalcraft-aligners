//! A golden case of `golden/fixtures/<case>/` in arch coordinates: the scan
//! moved by the inverse of the ground-truth arch frame (as the oracle's
//! `Case.V_arch`), the oracle's segmentation labels and its teeth.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use dc_case::Fdi;
use dca_algo::frame;
use glam::{DMat4, DVec3};
use serde_json::Value;

pub const CASES: [&str; 6] = [
    "demo_lower_mild",
    "demo_lower_moderate",
    "demo_lower_severe",
    "demo_upper_mild",
    "demo_upper_moderate",
    "demo_upper_severe",
];

pub fn dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden/fixtures")
        .join(name)
}

pub fn json(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

pub fn mat4(v: &Value) -> DMat4 {
    let r: Vec<Vec<f64>> = v
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap())
                .collect()
        })
        .collect();
    DMat4::from_cols_array_2d(&[
        [r[0][0], r[1][0], r[2][0], r[3][0]],
        [r[0][1], r[1][1], r[2][1], r[3][1]],
        [r[0][2], r[1][2], r[2][2], r[3][2]],
        [r[0][3], r[1][3], r[2][3], r[3][3]],
    ])
}

pub fn vec3(v: &Value) -> DVec3 {
    let a = v.as_array().unwrap();
    DVec3::new(
        a[0].as_f64().unwrap(),
        a[1].as_f64().unwrap(),
        a[2].as_f64().unwrap(),
    )
}

/// `mu.transform_points(m, p)`: `p @ m[:3, :3].T + m[:3, 3]`.
pub fn transform(m: &DMat4, p: DVec3) -> DVec3 {
    let c = m.to_cols_array_2d();
    DVec3::new(
        p.x * c[0][0] + p.y * c[1][0] + p.z * c[2][0] + c[3][0],
        p.x * c[0][1] + p.y * c[1][1] + p.z * c[2][1] + c[3][1],
        p.x * c[0][2] + p.y * c[1][2] + p.z * c[2][2] + c[3][2],
    )
}

pub fn unpack_labels(v: &Value) -> Vec<u16> {
    let n = v["n"].as_u64().unwrap() as usize;
    let rle = v["rle"].as_array().unwrap();
    let mut out = Vec::with_capacity(n);
    for pair in rle.chunks(2) {
        let (l, c) = (pair[0].as_u64().unwrap(), pair[1].as_u64().unwrap());
        out.extend(std::iter::repeat_n(l as u16, c as usize));
    }
    assert_eq!(out.len(), n);
    out
}

pub struct Case {
    pub name: String,
    pub dir: PathBuf,
    pub to_case: DMat4,
    pub to_arch: DMat4,
    pub vertices: Vec<DVec3>,
    pub tris: Vec<[u32; 3]>,
    pub labels: Vec<u16>,
    pub teeth: Value,
}

impl Case {
    pub fn load(name: &str) -> Self {
        let d = dir(name);
        let meta = json(&d.join("meta.json"));
        let to_case = mat4(&meta["frame_gt"]);
        let to_arch = frame::rigid_inverse(&to_case);
        let scan = dc_mesh::dcm::decode(&std::fs::read(d.join("scan.dcm")).unwrap()).unwrap();
        let vertices = scan
            .positions
            .iter()
            .map(|p| transform(&to_arch, p.as_dvec3()))
            .collect();
        let seg = json(&d.join("segmentation/segmentation.json"));
        let labels = unpack_labels(&seg["vertex_labels"]);
        let teeth = json(&d.join("teeth/teeth.json"));
        Self {
            name: name.to_owned(),
            dir: d,
            to_case,
            to_arch,
            vertices,
            tris: scan.tris.clone(),
            labels,
            teeth,
        }
    }

    pub fn fdis(&self) -> Vec<Fdi> {
        self.teeth["sequence"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| Fdi::new(f.as_u64().unwrap() as u8).unwrap())
            .collect()
    }

    pub fn axes(&self) -> BTreeMap<Fdi, DVec3> {
        self.fdis()
            .into_iter()
            .map(|f| (f, vec3(&self.teeth["teeth"][f.to_string()]["e_ax"])))
            .collect()
    }

    pub fn rest_frame(&self, f: Fdi) -> DMat4 {
        mat4(&self.teeth["teeth"][f.to_string()]["rest_frame"])
    }
}
