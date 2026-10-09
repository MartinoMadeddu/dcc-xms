//! Curves and points as lines: what the viewport draws of them, and how
//! curves of different kinds are merged.
//!
//! Curves keep their control points and their basis (Bézier, B-spline,
//! Catmull-Rom or linear) as USD gives them; they are evaluated into
//! polylines only to be drawn, a few steps per segment. Points are drawn as
//! small crosses, their width when they have one.

use bevy::math::Vec3;

use crate::types::{CurveBasis, CurveWrap, MeshData};

/// Steps per cubic segment when drawing.
const STEPS: usize = 6;

/// Above this many control points, curves are drawn by their control
/// polylines: a groom of millions of curves stays light, and its shape still
/// reads.
const DENSE: usize = 2_000_000;

/// One curve as a polyline through `steps` points per segment.
pub fn polyline(cv: &[Vec3], basis: CurveBasis, wrap: CurveWrap, steps: usize) -> Vec<Vec3> {
    if cv.len() < 2 { return cv.to_vec(); }
    let periodic = wrap == CurveWrap::Periodic;
    match basis {
        CurveBasis::Linear => {
            let mut out = cv.to_vec();
            if periodic { out.push(cv[0]); }
            out
        }
        CurveBasis::Bezier => {
            // Segments of four control points, each starting where the last ended.
            let mut pts = cv.to_vec();
            if periodic { pts.push(cv[0]); }
            if pts.len() < 4 { return pts; }
            let mut out = vec![pts[0]];
            let mut i = 0;
            while i + 3 < pts.len() {
                let [a, b, c, d] = [pts[i], pts[i + 1], pts[i + 2], pts[i + 3]];
                for k in 1..=steps {
                    let t = k as f32 / steps as f32;
                    let u = 1.0 - t;
                    out.push(a * (u * u * u) + b * (3.0 * u * u * t) + c * (3.0 * u * t * t) + d * (t * t * t));
                }
                i += 3;
            }
            out
        }
        CurveBasis::Bspline | CurveBasis::CatmullRom => {
            // Windows of four control points, one step apart. Periodic curves
            // repeat their first three; pinned ones get a phantom point at
            // each end, as USD defines them.
            let mut pts: Vec<Vec3> = Vec::with_capacity(cv.len() + 3);
            if wrap == CurveWrap::Pinned { pts.push(cv[0] * 2.0 - cv[1]); }
            pts.extend_from_slice(cv);
            if wrap == CurveWrap::Pinned { pts.push(cv[cv.len() - 1] * 2.0 - cv[cv.len() - 2]); }
            if periodic { pts.extend_from_slice(&cv[..3.min(cv.len())]); }
            if pts.len() < 4 { return cv.to_vec(); }
            let mut out = vec![];
            for w in pts.windows(4) {
                let first = if out.is_empty() { 0 } else { 1 };
                for k in first..=steps {
                    let t = k as f32 / steps as f32;
                    out.push(if basis == CurveBasis::Bspline { bspline(w, t) } else { catmull_rom(w, t) });
                }
            }
            out
        }
    }
}

fn bspline(p: &[Vec3], t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    let u = 1.0 - t;
    (p[0] * (u * u * u) + p[1] * (3.0 * t3 - 6.0 * t2 + 4.0) + p[2] * (-3.0 * t3 + 3.0 * t2 + 3.0 * t + 1.0) + p[3] * t3) / 6.0
}

fn catmull_rom(p: &[Vec3], t: f32) -> Vec3 {
    let (t2, t3) = (t * t, t * t * t);
    (p[1] * 2.0 + (p[2] - p[0]) * t + (p[0] * 2.0 - p[1] * 5.0 + p[2] * 4.0 - p[3]) * t2 + (p[1] * 3.0 - p[0] - p[2] * 3.0 + p[3]) * t3) * 0.5
}

/// Each curve's control points, by curve.
fn curves(md: &MeshData) -> impl Iterator<Item = &[[f32; 3]]> + '_ {
    let mut at = 0usize;
    md.curve_counts.iter().filter_map(move |&n| {
        let (start, end) = (at, at + n as usize);
        at = end;
        md.curve_points.get(start..end)
    })
}

/// The curves as linear polylines: for merging curves of different kinds.
pub fn as_polylines(md: &MeshData) -> (Vec<[f32; 3]>, Vec<u32>) {
    if md.curve_basis == CurveBasis::Linear && md.curve_wrap != CurveWrap::Periodic {
        return (md.curve_points.clone(), md.curve_counts.clone());
    }
    let (mut points, mut counts) = (vec![], vec![]);
    for cv in curves(md) {
        let cv: Vec<Vec3> = cv.iter().map(|p| Vec3::from_array(*p)).collect();
        let line = polyline(&cv, md.curve_basis, md.curve_wrap, STEPS);
        counts.push(line.len() as u32);
        points.extend(line.iter().map(|p| p.to_array()));
    }
    (points, counts)
}

/// Whether there is anything to draw as lines.
pub fn has_strands(md: &MeshData) -> bool {
    !md.curve_counts.is_empty() || !md.points.is_empty()
}

/// Curves and points as line segments: positions and pairs of indices.
pub fn lines(md: &MeshData) -> (Vec<[f32; 3]>, Vec<u32>) {
    let (mut pos, mut idx): (Vec<[f32; 3]>, Vec<u32>) = (vec![], vec![]);
    let dense = md.curve_points.len() > DENSE;
    for cv in curves(md) {
        let line: Vec<Vec3> = {
            let cv: Vec<Vec3> = cv.iter().map(|p| Vec3::from_array(*p)).collect();
            if dense { cv } else { polyline(&cv, md.curve_basis, md.curve_wrap, STEPS) }
        };
        if line.len() < 2 { continue; }
        let base = pos.len() as u32;
        pos.extend(line.iter().map(|p| p.to_array()));
        for k in 0..line.len() as u32 - 1 { idx.extend([base + k, base + k + 1]); }
    }
    if !md.points.is_empty() {
        // Points without a width: a small fraction of the cloud's size.
        let (lo, hi) = md.points.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), p| {
            let p = Vec3::from_array(*p);
            (lo.min(p), hi.max(p))
        });
        let fallback = ((hi - lo).length() * 0.003).max(1e-4);
        for (i, p) in md.points.iter().enumerate() {
            let w = md.widths.get(i).or(md.widths.first()).copied().unwrap_or(0.0);
            let r = if w > 0.0 { w * 0.5 } else { fallback };
            let c = Vec3::from_array(*p);
            for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
                let base = pos.len() as u32;
                pos.extend([(c - axis * r).to_array(), (c + axis * r).to_array()]);
                idx.extend([base, base + 1]);
            }
        }
    }
    (pos, idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bezier_segment_ends_on_its_end_points() {
        let cv = [Vec3::ZERO, Vec3::new(1.0, 1.0, 0.0), Vec3::new(2.0, 1.0, 0.0), Vec3::new(3.0, 0.0, 0.0)];
        let line = polyline(&cv, CurveBasis::Bezier, CurveWrap::Nonperiodic, 4);
        assert_eq!(line.len(), 5);
        assert_eq!(line[0], cv[0]);
        assert!((line[4] - cv[3]).length() < 1e-6);
    }

    #[test]
    fn a_catmull_rom_curve_passes_through_its_inner_points() {
        let cv = [Vec3::ZERO, Vec3::X, Vec3::new(2.0, 1.0, 0.0), Vec3::new(3.0, 1.0, 0.0)];
        let line = polyline(&cv, CurveBasis::CatmullRom, CurveWrap::Nonperiodic, 4);
        assert!((line[0] - cv[1]).length() < 1e-6 && (line[4] - cv[2]).length() < 1e-6);
    }

    #[test]
    fn a_pinned_bspline_reaches_its_ends() {
        let cv = [Vec3::ZERO, Vec3::X, Vec3::new(2.0, 1.0, 0.0), Vec3::new(3.0, 0.0, 0.0)];
        let line = polyline(&cv, CurveBasis::Bspline, CurveWrap::Pinned, 4);
        // The phantom points pull the curve's ends to within a sixth of the
        // first and last spans, as uniform B-splines do.
        assert!((line[0] - cv[0]).length() < 0.2 && (line[line.len() - 1] - cv[3]).length() < 0.2);
    }

    #[test]
    fn curves_and_points_become_segments() {
        let md = MeshData {
            curve_points: vec![[0.0; 3], [1.0, 0.0, 0.0], [2.0, 0.0, 0.0]],
            curve_counts: vec![3],
            points: vec![[0.0, 5.0, 0.0]],
            widths: vec![0.2],
            ..Default::default()
        };
        let (pos, idx) = lines(&md);
        // Two segments for the linear curve, three for the cross.
        assert_eq!(idx.len() / 2, 2 + 3);
        assert!((pos[3][0] - (-0.1)).abs() < 1e-6);
    }
}
