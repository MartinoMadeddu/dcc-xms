//! The solver on small made-up scenes with known answers.

use std::sync::Arc;
use glam::{Quat, Vec3};
use xms_ragdoll::{bake, BakeInput, BodyDef, Bvh, Hull, Params, Pose, Solver};

fn capsule_body(name: &str, parent: Option<usize>, len: f32, radius: f32, core: bool) -> BodyDef {
    // The bone runs along +X from its joint, as limbs usually do.
    let hull = Hull::capsule(Vec3::X * radius, Vec3::X * (len - radius), radius, 2);
    BodyDef {
        name: name.into(), parent, mass: hull.volume * 1000.0, hull,
        follow: 0.4, follow_turn: 0.4, trunk: false, turn_resist: 1.0, swing: 1.2, twist: 0.8, twist_axis: Vec3::X, hinge: None, range: None, core, sink: 0.0,
    }
}

/// A square sheet in the XZ plane at height y.
fn sheet(y: f32, size: f32, n: usize) -> Arc<Bvh> {
    let mut v = vec![];
    let mut idx = vec![];
    for j in 0..=n { for i in 0..=n { v.push([(i as f32 / n as f32 - 0.5) * size, y, (j as f32 / n as f32 - 0.5) * size]); } }
    let w = (n + 1) as u32;
    for j in 0..n as u32 { for i in 0..n as u32 { let a = j * w + i; idx.extend([a, a + w, a + 1, a + 1, a + w, a + w + 1]); } }
    Arc::new(Bvh::new(&v, &idx))
}

/// A closed box.
fn block(lo: Vec3, hi: Vec3) -> Arc<Bvh> {
    let v: Vec<[f32; 3]> = (0..8).map(|i| [if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z }]).collect();
    let idx = [0, 2, 1, 1, 2, 3, 4, 5, 6, 5, 7, 6, 0, 1, 4, 1, 5, 4, 2, 6, 3, 3, 6, 7, 0, 4, 2, 2, 4, 6, 1, 3, 5, 3, 7, 5];
    Arc::new(Bvh::new(&v, &idx))
}

fn run(solver: &mut Solver, frames: usize, target: &(dyn Fn(usize) -> Vec<Pose> + Sync)) -> (Vec<Vec<Pose>>, xms_ragdoll::BakeReport, Vec<f32>) {
    let mut out = vec![];
    let mut weights = vec![];
    let report = bake(solver, &BakeInput { frames, chunk: 16, target }, &mut |chunk| {
        for f in chunk { out.push(f.poses.clone()); weights.push(f.stats.weight); }
        true
    });
    (out, report, weights)
}

#[test]
fn with_nothing_in_the_way_the_capture_comes_back_exactly() {
    let defs = vec![capsule_body("upper", None, 0.3, 0.05, true), capsule_body("lower", Some(0), 0.3, 0.04, false)];
    let target = |f: usize| -> Vec<Pose> {
        let t = f as f32 / 30.0;
        let q0 = Quat::from_rotation_z(t.sin()) * Quat::from_rotation_y(0.5 * (2.0 * t).cos());
        let p0 = Vec3::new(t, 1.0 + 0.2 * (3.0 * t).sin(), 0.3 * t);
        let q1 = q0 * Quat::from_rotation_z(0.8 * (4.0 * t).sin());
        vec![Pose { p: p0, q: q0 }, Pose { p: p0 + q0 * Vec3::X * 0.3, q: q1 }]
    };
    let mut solver = Solver::new(defs, Params::default(), &target(0), None);
    let (out, report, _) = run(&mut solver, 120, &target);
    assert_eq!(out.len(), 120);
    for (f, poses) in out.iter().enumerate() {
        for (a, b) in poses.iter().zip(target(f)) {
            assert!((a.p - b.p).length() < 2e-4, "frame {f}: {} off", (a.p - b.p).length());
            assert!(a.q.angle_between(b.q) < 2e-3, "frame {f}");
        }
    }
    assert_eq!((report.ghost_frames, report.resets, report.released), (0, 0, 0));
}

#[test]
fn a_body_lowered_through_a_floor_rests_on_it_and_returns() {
    // A limb, so it is pushed out and not let through.
    let mut def = capsule_body("limb", None, 0.3, 0.05, false);
    def.follow = 0.4;
    // Down through the floor to 12 cm below, held, and up again.
    let height = |f: usize| -> f32 {
        let t = f as f32;
        if t < 30.0 { 0.3 - 0.42 * t / 30.0 } else if t < 60.0 { -0.12 } else { -0.12 + 0.42 * ((t - 60.0) / 30.0).min(1.0) }
    };
    let target = |f: usize| vec![Pose { p: Vec3::new(-0.15, height(f), 0.0), q: Quat::IDENTITY }];
    let mut solver = Solver::new(vec![def], Params::default(), &target(0), Some(sheet(0.0, 4.0, 20)));
    let (out, report, _) = run(&mut solver, 110, &target);
    let lowest = |p: &Pose| p.p.y - 0.05;
    for (f, poses) in out.iter().enumerate() {
        assert!(lowest(&poses[0]) > -0.004, "frame {f}: {} below the floor", -lowest(&poses[0]));
    }
    // Held: resting on the floor, inside the soft margin.
    assert!(lowest(&out[55][0]) < 0.02, "{}", lowest(&out[55][0]));
    // Back on the capture a little after it is clear.
    assert!((out[109][0].p.y - height(109)).abs() < 0.003, "{}", (out[109][0].p.y - height(109)).abs());
    // No jumps: the offset from the capture changes by little from frame to frame.
    let offset: Vec<f32> = out.iter().enumerate().map(|(f, p)| p[0].p.y - height(f)).collect();
    let jump = offset.windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0, f32::max);
    assert!(jump < 0.025, "offset jumps {jump}");
    assert!(report.max_residual < 0.004 && report.contact_frames > 30);
}

#[test]
fn a_blocked_forearm_bends_the_elbow_and_no_bone_stretches() {
    let upper = capsule_body("upper", None, 0.3, 0.05, true);
    let mut lower = capsule_body("lower", Some(0), 0.3, 0.04, false);
    lower.follow = 0.35;
    // The upper arm is level at 0.4 m. The forearm swings down from level
    // to 60 degrees below, which would put the wrist under the table at 0.25 m.
    let angle = |f: usize| -> f32 { -(f as f32 / 40.0).min(1.0) * 60f32.to_radians() };
    let target = |f: usize| -> Vec<Pose> {
        let p0 = Vec3::new(0.0, 0.4, 0.0);
        vec![Pose { p: p0, q: Quat::IDENTITY }, Pose { p: p0 + Vec3::X * 0.3, q: Quat::from_rotation_z(angle(f)) }]
    };
    let table = {
        // A small table top under the forearm only.
        let v = [[0.25, 0.25, -0.5], [1.2, 0.25, -0.5], [1.2, 0.25, 0.5], [0.25, 0.25, 0.5]];
        Arc::new(Bvh::new(&v, &[0, 2, 1, 0, 3, 2]))
    };
    let mut solver = Solver::new(vec![upper, lower], Params::default(), &target(0), Some(table));
    let (out, report, _) = run(&mut solver, 70, &target);
    let last = &out[69];
    // The wrist end of the forearm is on the table, not under it.
    let wrist = last[1].p + last[1].q * Vec3::X * 0.26;
    assert!(wrist.y - 0.04 > 0.25 - 0.004, "wrist at {}", wrist.y);
    // The elbow gave: the forearm is bent less than the capture asks.
    let bend = (last[1].q * Vec3::X).y.asin();
    assert!(bend > angle(69) + 0.3, "bend {bend}");
    // The upper arm moved much less than the forearm.
    let upper_off = last[0].q.angle_between(Quat::IDENTITY);
    assert!(upper_off < 0.25 * (bend - angle(69)).abs(), "upper arm turned {upper_off}");
    // The joint holds: elbow where the upper arm ends, in every frame.
    for poses in &out {
        let gap = (poses[0].p + poses[0].q * Vec3::X * 0.3 - poses[1].p).length();
        assert!(gap < 0.002, "joint open by {gap}");
    }
    assert_eq!(report.ghost_frames, 0);
}

#[test]
fn a_hinge_bends_about_its_axis_only_and_not_backwards() {
    use xms_ragdoll::Hinge;
    let upper = capsule_body("upper", None, 0.3, 0.05, true);
    let mut lower = capsule_body("lower", Some(0), 0.3, 0.04, false);
    lower.swing = 0.1;
    lower.hinge = Some(Hinge { axis: Vec3::Z, reference: Quat::IDENTITY, min: 0.0, max: 2.6 });
    // The capture holds the arm straight. A wall comes at the forearm from
    // the side the elbow cannot bend to.
    let target = |_: usize| -> Vec<Pose> { vec![Pose { p: Vec3::new(0.0, 0.4, 0.0), q: Quat::IDENTITY }, Pose { p: Vec3::new(0.3, 0.4, 0.0), q: Quat::IDENTITY }] };
    let above = { let v = [[0.3, 0.42, -0.5], [1.2, 0.42, -0.5], [1.2, 0.42, 0.5], [0.3, 0.42, 0.5]]; Arc::new(Bvh::new(&v, &[0, 1, 2, 0, 2, 3])) };
    let mut solver = Solver::new(vec![upper.clone(), lower.clone()], Params::default(), &target(0), Some(above));
    let (out, _, _) = run(&mut solver, 40, &target);
    // Pushed down means bending backwards (negative about Z): the hinge refuses,
    // so the whole arm tips instead of the elbow folding.
    let rel = out[39][0].q.inverse() * out[39][1].q;
    assert!(xms_ragdoll::solver::twist_angle(rel, Vec3::Z) > -0.03, "bent backwards by {}", xms_ragdoll::solver::twist_angle(rel, Vec3::Z));
    // Pushed up at the wrist end, by a block, it may bend, and does.
    let below = block(Vec3::new(0.5, 0.2, -0.5), Vec3::new(1.2, 0.385, 0.5));
    let mut solver = Solver::new(vec![upper, lower], Params::default(), &target(0), Some(below));
    let (out, _, _) = run(&mut solver, 40, &target);
    let rel = out[39][0].q.inverse() * out[39][1].q;
    assert!(xms_ragdoll::solver::twist_angle(rel, Vec3::Z) > 0.03, "did not bend: {}", xms_ragdoll::solver::twist_angle(rel, Vec3::Z));
}

#[test]
fn parts_of_one_character_do_not_pass_through_each_other() {
    // Two free limbs. One sweeps across the other.
    let a = capsule_body("a", None, 0.4, 0.05, true);
    let mut b = capsule_body("b", None, 0.4, 0.05, false);
    b.follow = 0.3;
    let target = |f: usize| -> Vec<Pose> {
        let y = 0.5 - (f as f32 / 40.0).min(1.0) * 0.5;      // b comes down onto a, to the same place
        vec![Pose { p: Vec3::ZERO, q: Quat::IDENTITY }, Pose { p: Vec3::new(0.2, y, -0.2), q: Quat::from_rotation_y(-1.5708) }]
    };
    let mut solver = Solver::new(vec![a, b], Params::default(), &target(0), None);
    let (out, _, _) = run(&mut solver, 60, &target);
    // The axes of the two capsules cross at right angles: their distance is the difference in height.
    let gap = out[59][1].p.y - out[59][0].p.y;
    assert!(gap > 0.1 - 0.03 - 0.01, "axes {gap} apart, capsules are 0.1 thick together");
    // Without self collision they end in the same place.
    let mut p = Params::default();
    p.self_collision = false;
    let defs = vec![capsule_body("a", None, 0.4, 0.05, true), capsule_body("b", None, 0.4, 0.05, false)];
    let mut solver = Solver::new(defs, p, &target(0), None);
    let (out, _, _) = run(&mut solver, 60, &target);
    assert!((out[59][1].p.y - out[59][0].p.y).abs() < 0.01);
}

#[test]
fn a_character_walked_through_a_wall_follows_the_capture_through() {
    // The trunk walks along X through a wall at x = 0, with an arm in tow.
    let trunk = capsule_body("trunk", None, 0.5, 0.15, true);
    let arm = capsule_body("arm", Some(0), 0.5, 0.05, false);
    let target = |f: usize| -> Vec<Pose> {
        let x = -1.5 + f as f32 * 0.03;
        let up = Quat::from_rotation_z(1.5708);
        vec![Pose { p: Vec3::new(x, 0.9, 0.0), q: up }, Pose { p: Vec3::new(x, 1.3, 0.2), q: Quat::from_rotation_z(-1.5708) }]
    };
    let wall = {
        let v = [[0.0, 0.0, -2.0], [0.0, 3.0, -2.0], [0.0, 3.0, 2.0], [0.0, 0.0, 2.0]];
        Arc::new(Bvh::new(&v, &[0, 1, 2, 0, 2, 3]))
    };
    let mut solver = Solver::new(vec![trunk, arm], Params::default(), &target(0), Some(wall));
    let (out, report, weights) = run(&mut solver, 100, &target);
    // It gets through: at the end it is on the capture, on the far side.
    assert!(out[99][0].p.x > 1.0 && (out[99][0].p - target(99)[0].p).length() < 0.002);
    // While the trunk is in the wall, collisions are off and it is on the capture.
    let crossing = 50;      // x = 0
    assert_eq!(weights[crossing], 0.0);
    assert!((out[crossing][0].p - target(crossing)[0].p).length() < 0.01);
    // Off before the wall is reached and on again after, by degrees.
    assert!(weights[20] == 1.0 && weights[95] == 1.0);
    assert!(weights.windows(2).all(|w| (w[1] - w[0]).abs() <= 1.0 / 5.0 + 1e-4), "weight jumps");
    assert_eq!(report.ghost_ranges.len(), 1);
    let (a, b) = report.ghost_ranges[0];
    assert!(a < crossing && b > crossing && b - a < 60, "ghost from {a} to {b}");
    // And it never leaves the capture by much: it is not shoved along the wall.
    assert!(report.max_deviation < 0.12, "pushed {} from the capture", report.max_deviation);
}

#[test]
fn bad_input_cannot_break_it() {
    let defs = vec![capsule_body("a", None, 0.3, 0.05, true), capsule_body("b", Some(0), 0.3, 0.04, false)];
    // Jumps of many metres, a flip, and a stretch where the two bones are pulled apart.
    let target = |f: usize| -> Vec<Pose> {
        let jump = if f % 7 == 0 { 25.0 } else { 0.0 };
        let p0 = Vec3::new(jump, 0.05, 0.0);
        let q0 = if f % 5 == 0 { Quat::from_rotation_x(3.1) } else { Quat::IDENTITY };
        vec![Pose { p: p0, q: q0 }, Pose { p: p0 + Vec3::X * if f % 11 == 0 { 3.0 } else { 0.3 }, q: Quat::from_rotation_z(f as f32) }]
    };
    let mut solver = Solver::new(defs, Params::default(), &target(0), Some(sheet(0.0, 100.0, 30)));
    let (out, _, _) = run(&mut solver, 80, &target);
    for (f, poses) in out.iter().enumerate() {
        for (a, b) in poses.iter().zip(target(f)) {
            assert!(a.p.is_finite() && a.q.is_finite());
            assert!((a.p - b.p).length() < 2.1, "frame {f}: {} from the capture", (a.p - b.p).length());
        }
    }
}

#[test]
fn memory_is_one_chunk_whatever_the_length() {
    // The capture is asked for each frame once, in order, never far ahead of
    // what has been handed over.
    use std::sync::atomic::{AtomicUsize, Ordering};
    let defs = vec![capsule_body("a", None, 0.3, 0.05, true)];
    let (asked, highest) = (AtomicUsize::new(0), AtomicUsize::new(0));
    let target = |f: usize| -> Vec<Pose> {
        asked.fetch_add(1, Ordering::Relaxed);
        highest.fetch_max(f, Ordering::Relaxed);
        vec![Pose { p: Vec3::new(f as f32 * 0.01, 1.0, 0.0), q: Quat::IDENTITY }]
    };
    let mut solver = Solver::new(defs, Params::default(), &target(0), None);
    asked.store(0, Ordering::Relaxed);
    let mut done = 0;
    let mut furthest_ahead = 0;
    let report = bake(&mut solver, &BakeInput { frames: 1000, chunk: 64, target: &target }, &mut |chunk| {
        done += chunk.len();
        furthest_ahead = furthest_ahead.max(highest.load(Ordering::Relaxed) + 1 - done);
        true
    });
    assert_eq!((report.frames, done), (1000, 1000));
    assert_eq!(asked.load(Ordering::Relaxed), 1000);
    let p = Params::default();
    assert!(furthest_ahead <= 4 * p.fade_out + 3 * p.fade_in + 2 + p.smooth, "{furthest_ahead} frames ahead");
    // Stopping from the sink stops.
    let mut solver = Solver::new(vec![capsule_body("a", None, 0.3, 0.05, true)], Params::default(), &target(0), None);
    let report = bake(&mut solver, &BakeInput { frames: 1000, chunk: 64, target: &target }, &mut |_| false);
    assert!(report.cancelled && report.frames == 64);
}

#[test]
fn a_trunk_sunk_deep_is_moved_only_so_far() {
    // A trunk lowered 20 cm into a floor, sideways on so its middle stays above.
    let mut trunk = capsule_body("trunk", None, 0.6, 0.25, true);
    trunk.follow = 0.7;
    let height = |f: usize| 0.30 - 0.25 * (f as f32 / 40.0).min(1.0);
    let target = |f: usize| vec![Pose { p: Vec3::new(-0.3, height(f), 0.0), q: Quat::IDENTITY }];
    let mut p = Params::default();
    p.ghost_depth = 0.05;
    let mut solver = Solver::new(vec![trunk], p, &target(0), Some(sheet(0.0, 6.0, 30)));
    let (out, report, _) = run(&mut solver, 80, &target);
    // The capture ends 20 cm in. The trunk is lifted 5 cm of that and no more.
    let lift = out[79][0].p.y - height(79);
    assert!((lift - 0.05).abs() < 0.02, "lifted {lift}");
    // And collisions were never given up: it did not pass through.
    assert_eq!(report.ghost_frames, 0);
}
