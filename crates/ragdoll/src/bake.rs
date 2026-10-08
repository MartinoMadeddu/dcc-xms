//! Running the solver over a whole take, a chunk of frames at a time.
//!
//! A long take is never held in full. For each chunk the capture is asked
//! for the frames of that chunk and the few after it that are needed to
//! look ahead, the chunk is solved and handed over, and its frames are
//! dropped. What stays in memory is one chunk, whatever the length of the
//! take.
//!
//! What the solver changed, the offset of each body from its capture, is
//! evened out over a few frames before it is handed over. A contact is
//! then taken up over those frames, and a joint that cannot settle
//! between two answers gets the mean of them.
//!
//! Looking ahead is what makes the failsafe smooth. Before a chunk is
//! solved, the capture alone is examined: where it takes the trunk of the
//! character through the collider, no solving can keep the character out.
//! Collisions are faded out before such a stretch and back in after it, so
//! the character follows the capture through and is caught again softly on
//! the other side.

use glam::{Quat, Vec3};
use crate::solver::{FrameStats, Pose, Solver};

pub struct BakeInput<'a> {
    pub frames: usize,
    /// Frames solved and handed over at a time.
    pub chunk:  usize,
    /// The capture: the pose of every body at a frame.
    pub target: &'a (dyn Fn(usize) -> Vec<Pose> + Sync),
}

pub struct FrameOut {
    pub frame: usize,
    pub poses: Vec<Pose>,
    pub stats: FrameStats,
}

#[derive(Clone, Debug, Default)]
pub struct BakeReport {
    pub frames:       usize,
    /// Frames in which collisions were off or fading, and the stretches they form.
    pub ghost_frames: usize,
    pub ghost_ranges: Vec<(usize, usize)>,
    /// Frames in which something touched or came near the collider.
    pub contact_frames: usize,
    pub max_residual: f32,
    pub max_deviation: f32,
    pub released:     usize,
    pub resets:       usize,
    pub substeps:     usize,
    pub cancelled:    bool,
}

/// Solve a take. `sink` receives each chunk as it is done and returns
/// false to stop.
pub fn bake(solver: &mut Solver, input: &BakeInput, sink: &mut dyn FnMut(&[FrameOut]) -> bool) -> BakeReport {
    let mut report = BakeReport::default();
    let frames = input.frames;
    if frames == 0 || solver.body_count() == 0 { return report; }
    let chunk = input.chunk.max(8);
    let (fade_out, fade_in) = (solver.params.fade_out.max(1), solver.params.fade_in.max(1));
    // Two blocked stretches closer together than this are one: collisions
    // are not brought back for a moment only to be taken away again.
    let bridge = 2 * fade_in + fade_out;
    // A ghost stretch reaches out over the frames around it in which the
    // trunk still touches the collider, this far at most, so collisions are
    // gone before the trunk is pushed and back after it is clear.
    let spread = 2 * fade_out + fade_in;
    let ahead = fade_out + bridge + spread + 2;
    let threads = solver.params.threads.max(1);

    // The window of capture frames in memory: `base` is the frame of window[0].
    let mut base = 0usize;
    let mut window: Vec<Vec<Pose>> = vec![];
    let mut blocked: Vec<bool> = vec![];
    let mut touching: Vec<bool> = vec![];
    let mut depths: Vec<Vec<f32>> = vec![];
    // Frame of the last blocked frame seen so far, for the fade back in.
    let mut last_blocked: Option<usize> = None;
    let mut last_ghost_frame: Option<usize> = None;
    let mut ghost_start: Option<usize> = None;

    // Solved frames waiting for the frames after them, to be evened out.
    let radius = solver.params.smooth.min(8);
    let mut held: std::collections::VecDeque<Held> = std::collections::VecDeque::new();
    // Offsets of the frames already handed over, newest last.
    let mut past: std::collections::VecDeque<Vec<(Vec3, Quat)>> = std::collections::VecDeque::new();

    let mut start = 0usize;
    while start < frames {
        let end = (start + chunk).min(frames);
        let need = (end + ahead).min(frames);
        // Fetch what is missing, several frames at a time.
        let have = base + window.len();
        if need > have {
            let fetched = fetch(input.target, have, need, threads);
            window.extend(fetched);
            // Blocked or not, for each new frame, from the capture alone.
            let flags = analyse(solver, &window, base, have, need, threads);
            blocked.extend(flags.iter().map(|f| f.0));
            touching.extend(flags.iter().map(|f| f.1));
            depths.extend(flags.into_iter().map(|f| f.2));
        }
        let pose = |f: usize| -> &Vec<Pose> { &window[f.clamp(base, base + window.len() - 1) - base] };
        // Frames the character is a ghost in: the blocked ones, and those
        // between two blocked frames no further apart than the bridge.
        let top = base + blocked.len();
        let mut ghost: Vec<bool> = blocked.clone();
        let mut prev_blocked = last_blocked;
        for f in base.max(start)..top {
            if blocked[f - base] {
                if let Some(p) = prev_blocked { if f - p <= bridge { for g in p.max(base)..f { ghost[g - base] = true; } } }
                prev_blocked = Some(f);
            }
        }
        // Out over the touching frames on both sides.
        let core = ghost.clone();
        for f in base..top {
            if !core[f - base] { continue; }
            for k in 1..=spread {
                if f < base + k || core[f - k - base] || !touching[f - k - base] { break; }
                ghost[f - k - base] = true;
            }
            for k in 1..=spread {
                if f + k >= top || core[f + k - base] || !touching[f + k - base] { break; }
                ghost[f + k - base] = true;
            }
        }
        let weight_at = |f: usize, last: Option<usize>| -> f32 {
            if ghost[f - base] { return 0.0; }
            let mut w = 1.0f32;
            // Fade out toward the next ghost frame.
            for k in 1..=fade_out {
                let g = f + k;
                if g < top && ghost[g - base] { w = w.min((k - 1) as f32 / fade_out as f32); break; }
            }
            // Fade in from the last one.
            if let Some(l) = last { if f > l { w = w.min((f - l - 1) as f32 / fade_in as f32); } }
            w.clamp(0.0, 1.0)
        };

        let mut out: Vec<FrameOut> = Vec::with_capacity(end - start);
        // The last ghost frame at or before each frame, for the fade back in.
        let mut last_ghost = last_ghost_frame;
        for f in start..end {
            if blocked[f - base] { last_blocked = Some(f); }
            let before = last_ghost;
            if ghost[f - base] { last_ghost = Some(f); }
            let w1 = weight_at(f, last_ghost);
            // How deep the trunk is left in: what its capture has beyond the
            // furthest it may be moved, evened over the frames around.
            let reach = solver.params.ghost_depth;
            let (lo, hi) = (f.saturating_sub(3).max(base), (f + 4).min(top));
            let mut left = vec![0.0f32; depths[f - base].len()];
            for g in lo..hi { for (l, d) in left.iter_mut().zip(&depths[g - base]) { *l += (d - reach).max(0.0); } }
            for l in left.iter_mut() { *l /= (hi - lo).max(1) as f32; }
            solver.leave_in(&left);
            let stats = if f == 0 {
                solver.reset(pose(0));
                // The first frame is solved from itself until it has settled,
                // so a take that starts in the collider starts out of it.
                let p0 = pose(0).clone();
                let mut s = FrameStats::default();
                for _ in 0..60 {
                    s = solver.step(&p0, &p0, &p0, &p0, w1, w1);
                    if s.residual < 0.002 { break; }
                }
                s
            } else {
                let w0 = if f - 1 >= base { weight_at(f - 1, before) } else { w1 };
                solver.step(pose(f.saturating_sub(2)), pose(f - 1), pose(f), pose(f + 1), w0, w1)
            };
            report.frames += 1;
            report.substeps += stats.substeps as usize;
            report.released += stats.released as usize;
            report.resets += stats.resets as usize;
            report.max_residual = report.max_residual.max(stats.residual);
            report.max_deviation = report.max_deviation.max(stats.deviation);
            if stats.contacts > 0 { report.contact_frames += 1; }
            if w1 < 1.0 {
                report.ghost_frames += 1;
                if ghost_start.is_none() { ghost_start = Some(f); }
            } else if let Some(s) = ghost_start.take() {
                report.ghost_ranges.push((s, f - 1));
            }
            let solved = solver.poses();
            let target = pose(f);
            // What the solver changed: how far each body is from its capture, and how it is turned from it.
            let offset: Vec<(Vec3, Quat)> = solved.iter().zip(target.iter()).map(|(s, t)| (s.p - t.p, (s.q * t.q.inverse()).normalize())).collect();
            held.push_back(Held { frame: f, target: target.clone(), offset, stats });
            // A frame goes out when the frames it is evened with are there.
            while held.len() > radius || (f + 1 == frames && !held.is_empty()) {
                out.push(even_out(&held, &past, radius));
                let done = held.pop_front().unwrap();
                past.push_back(done.offset);
                if past.len() > radius { past.pop_front(); }
            }
        }
        last_ghost_frame = last_ghost;
        let go_on = sink(&out);
        drop(out);
        start = end;
        if !go_on { report.cancelled = true; break; }
        // Drop the frames that are done, but for the two the next chunk starts from.
        let keep_from = start.saturating_sub(2);
        if keep_from > base {
            let cut = keep_from - base;
            window.drain(..cut);
            blocked.drain(..cut);
            touching.drain(..cut);
            depths.drain(..cut);
            base = keep_from;
        }
    }
    if let Some(s) = ghost_start { report.ghost_ranges.push((s, report.frames.saturating_sub(1))); }
    report
}

fn fetch(target: &(dyn Fn(usize) -> Vec<Pose> + Sync), from: usize, to: usize, threads: usize) -> Vec<Vec<Pose>> {
    let n = to - from;
    if threads <= 1 || n < 8 { return (from..to).map(target).collect(); }
    let per = n.div_ceil(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads).map(|t| {
            let (a, b) = ((from + t * per).min(to), (from + (t + 1) * per).min(to));
            scope.spawn(move || (a..b).map(target).collect::<Vec<_>>())
        }).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

fn analyse(solver: &Solver, window: &[Vec<Pose>], base: usize, from: usize, to: usize, threads: usize) -> Vec<(bool, bool, Vec<f32>)> {
    let one = |f: usize| -> (bool, bool, Vec<f32>) {
        let to_pose = &window[f - base];
        let from_pose = if f > base { &window[f - 1 - base] } else { to_pose };
        solver.core_state(from_pose, to_pose)
    };
    let n = to - from;
    if threads <= 1 || n < 8 { return (from..to).map(one).collect(); }
    let per = n.div_ceil(threads);
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads).map(|t| {
            let (a, b) = ((from + t * per).min(to), (from + (t + 1) * per).min(to));
            let one = &one;
            scope.spawn(move || (a..b).map(one).collect::<Vec<_>>())
        }).collect();
        handles.into_iter().flat_map(|h| h.join().unwrap_or_default()).collect()
    })
}

struct Held {
    frame:  usize,
    target: Vec<Pose>,
    offset: Vec<(Vec3, Quat)>,
    stats:  FrameStats,
}

/// The first held frame, its offsets from the capture evened with those of
/// the frames around it: a weighted mean, the frame itself counting most.
fn even_out(held: &std::collections::VecDeque<Held>, past: &std::collections::VecDeque<Vec<(Vec3, Quat)>>, radius: usize) -> FrameOut {
    let now = &held[0];
    let mut poses = Vec::with_capacity(now.target.len());
    for b in 0..now.target.len() {
        let (own_p, own_q) = now.offset[b];
        let (mut p, mut q, mut total) = (own_p * (radius + 1) as f32, own_q * (radius + 1) as f32, (radius + 1) as f32);
        let mut add = |o: &(Vec3, Quat), w: f32| {
            p += o.0 * w;
            // The same rotation has two signs: take the one on the side of this frame's.
            q = q + if o.1.dot(own_q) < 0.0 { -o.1 } else { o.1 } * w;
            total += w;
        };
        for k in 1..=radius {
            let w = (radius + 1 - k) as f32;
            if let Some(h) = held.get(k) { add(&h.offset[b], w); }
            if past.len() >= k { add(&past[past.len() - k][b], w); }
        }
        let q = if q.length_squared() > 1e-12 { q.normalize() } else { own_q };
        poses.push(Pose { p: now.target[b].p + p / total, q: (q * now.target[b].q).normalize() });
    }
    FrameOut { frame: now.frame, poses, stats: now.stats }
}
