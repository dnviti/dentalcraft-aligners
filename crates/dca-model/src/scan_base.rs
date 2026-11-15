//! The base an open arch scan was closed with.
//!
//! An intraoral scan is open under the gum. The import closes it with a flat
//! base (`aligners.case.import-scans`), and so does "Close with a base"
//! (`aligners.case.make-base`, the plugin's `oa.make_base`): the scan of the
//! case becomes the closed solid, the one shown, saved and used by the view,
//! the bite and the export. The scan as it was read stays in it: its points
//! and triangles come first, in their order, and the base follows
//! (`dca_algo::scan_base::make_base`). [`ScanBase`] says where the scan ends,
//! so the steps that read the open border (finding the teeth, the
//! separation, the frame, the 3D teeth and gingiva) read the scan as it was
//! read (`dca_algo::scan_base::open_part`).

use dc_case::BlobRef;
use serde::{Deserialize, Serialize};

use crate::AlignersState;

/// Where the scan as it was read ends in a scan closed with a base: its
/// first `vertices` points and `faces` triangles; the points and triangles
/// after them are the base. When `flipped`, the scan was wound inwards and
/// every triangle of the closed solid was turned to face outwards, those of
/// the scan too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScanBase {
    /// Points of the scan as it was read.
    pub vertices: u32,
    /// Triangles of the scan as it was read.
    pub faces: u32,
    /// Every triangle was turned (its second and third corners swapped).
    pub flipped: bool,
    /// Who closed it, and on which direction the base lies.
    #[serde(default)]
    pub closed_by: ClosedBy,
}

/// Who closed a scan with a base, and on which direction (what the import
/// inspector says of the scan).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClosedBy {
    /// The import, perpendicular to the occlusal direction of the scan
    /// itself (it has no tooth marks yet).
    #[default]
    Import,
    /// "Close with a base" on an arch without its tooth marks: the same
    /// direction as the import.
    Hand,
    /// "Close with a base" on an arch with its tooth marks: perpendicular to
    /// the occlusal direction of their arch frame, as the plugin.
    ToothMarks,
}

impl ClosedBy {
    /// Its name in the reports of the commands.
    pub fn id(self) -> &'static str {
        match self {
            Self::Import => "import",
            Self::Hand => "hand",
            Self::ToothMarks => "tooth_marks",
        }
    }
}

impl AlignersState {
    /// Where the scan as it was read ends in the scan blob `blob`; `None`
    /// for a scan not closed with a base (open, or closed as it was read).
    pub fn scan_base(&self, blob: &BlobRef) -> Option<ScanBase> {
        self.scan_bases.get(blob).copied()
    }

    /// The scan blob `old` was moved rigidly (or changed point by point) and
    /// is now `new`, with the same points and triangles in the same order:
    /// the base it was closed with holds on `new`. The record of `old` stays
    /// for another scan with the same blob; [`AlignersState::prune_scan_bases`]
    /// drops it once no scan uses it.
    pub fn scan_moved(&mut self, old: BlobRef, new: BlobRef) {
        if let Some(b) = self.scan_base(&old) {
            self.scan_bases.insert(new, b);
        }
    }

    /// Drops the records of the blobs no scan of the case uses
    /// (`scan_blobs`: the meshes of its scans).
    pub fn prune_scan_bases<'a>(&mut self, scan_blobs: impl IntoIterator<Item = &'a BlobRef>) {
        let used: Vec<&BlobRef> = scan_blobs.into_iter().collect();
        self.scan_bases.retain(|b, _| used.contains(&b));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dc_mesh::TriMesh;
    use glam::Vec3;

    fn blob(z: f32) -> BlobRef {
        let mesh = TriMesh::new(vec![Vec3::Z * z, Vec3::X, Vec3::Y], vec![[0, 1, 2]]);
        BlobRef::of_mesh(&mesh)
    }

    #[test]
    fn the_record_follows_a_moved_scan_and_goes_with_the_last_scan_that_used_it() {
        let (a, b, c) = (blob(0.0), blob(1.0), blob(2.0));
        let base = ScanBase {
            vertices: 3,
            faces: 1,
            flipped: true,
            closed_by: ClosedBy::ToothMarks,
        };
        let mut d = AlignersState::default();
        d.scan_bases.insert(a, base);
        assert_eq!(d.scan_base(&a), Some(base));
        assert_eq!(d.scan_base(&b), None);
        // moved: the new blob has it, the old one too until it is pruned
        d.scan_moved(a, b);
        assert_eq!(d.scan_base(&b), Some(base));
        d.prune_scan_bases([&b, &c]);
        assert_eq!(d.scan_base(&a), None);
        assert_eq!(d.scan_base(&b), Some(base));
        // a blob without a record moves nothing
        d.scan_moved(c, a);
        assert_eq!(d.scan_base(&a), None);
    }

    #[test]
    fn it_is_saved_only_when_there_is_one_and_read_back() {
        let d = AlignersState::default();
        let v = serde_json::to_value(&d).unwrap();
        assert!(v.get("scan_bases").is_none(), "{v}");
        let mut d = AlignersState::default();
        d.scan_bases.insert(
            blob(0.0),
            ScanBase {
                vertices: 3,
                faces: 1,
                flipped: false,
                closed_by: ClosedBy::Hand,
            },
        );
        let v = serde_json::to_value(&d).unwrap();
        let records = v["scan_bases"].as_object().unwrap();
        assert_eq!(records.len(), 1, "{v}");
        let rec = records.values().next().unwrap();
        assert_eq!(rec["closed_by"], "hand", "{v}");
        let back: AlignersState = serde_json::from_value(v).unwrap();
        assert_eq!(back, d);
    }
}
