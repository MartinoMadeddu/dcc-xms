//! Raster canvas of the UV editor.
//!
//! The editor does not hand egui one line per UV edge: a heavy mesh has a
//! million of them. The canvas draws into a pixel buffer of its own, on
//! several threads, and the editor shows that buffer as one texture. It is
//! redrawn only when the view, the layout or an option changes.
//!
//! The canvas is a stack of layers composed into that one buffer. Today
//! there is a single layer, the wireframe. Paint layers are meant to go
//! under it later, each a buffer in UV space composed the same way.

/// One UV edge.
#[derive(Clone, Copy, Debug)]
pub struct Edge {
    pub a:      [f32; 2],
    pub b:      [f32; 2],
    /// On the border of its island.
    pub seam:   bool,
    pub island: u32,
}

/// Where UV space sits in the pixel buffer: `x = ox + u * scale`,
/// `y = oy - v * scale`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    pub w:     usize,
    pub h:     usize,
    pub scale: f32,
    pub ox:    f32,
    pub oy:    f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Options {
    /// Leave the edges inside islands out.
    pub borders_only: bool,
    pub selected:     Option<u32>,
    /// Draw seams two pixels wide, for dense displays.
    pub thick:        bool,
}

/// How much each line adds to a pixel it crosses. Lines piling up on one
/// pixel brighten it, so density reads as tone instead of a solid block.
const INNER_STEP: u8 = 56;

/// Coverage of the three kinds of line, one byte per pixel each.
struct Planes { inner: Vec<u8>, seam: Vec<u8>, picked: Vec<u8> }

impl Planes {
    fn new(n: usize) -> Self { Self { inner: vec![0; n], seam: vec![0; n], picked: vec![0; n] } }
}

/// The part of a segment inside the buffer, if any (Liang-Barsky). Worked
/// in double precision: a zoomed-in edge can run far outside the buffer.
fn clip(a: (f32, f32), b: (f32, f32), w: f32, h: f32) -> Option<((f32, f32), (f32, f32))> {
    let (ax, ay, bx, by) = (a.0 as f64, a.1 as f64, b.0 as f64, b.1 as f64);
    let (dx, dy) = (bx - ax, by - ay);
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    for (p, q) in [(-dx, ax), (dx, w as f64 - ax), (-dy, ay), (dy, h as f64 - ay)] {
        if p == 0.0 { if q < 0.0 { return None; } continue; }
        let t = q / p;
        if p < 0.0 { if t > t1 { return None; } t0 = t0.max(t); } else { if t < t0 { return None; } t1 = t1.min(t); }
    }
    Some((((ax + dx * t0) as f32, (ay + dy * t0) as f32), ((ax + dx * t1) as f32, (ay + dy * t1) as f32)))
}

fn draw(edges: &[Edge], view: &View, opt: &Options, planes: &mut Planes) {
    let (w, h) = (view.w, view.h);
    let (wf, hf) = (w as f32 - 1.0, h as f32 - 1.0);
    for e in edges {
        let chosen = opt.selected == Some(e.island);
        if opt.borders_only && !e.seam && !chosen { continue; }
        let a = (view.ox + e.a[0] * view.scale, view.oy - e.a[1] * view.scale);
        let b = (view.ox + e.b[0] * view.scale, view.oy - e.b[1] * view.scale);
        // Both ends off the same side: nothing to draw.
        if (a.0 < 0.0 && b.0 < 0.0) || (a.1 < 0.0 && b.1 < 0.0) || (a.0 > wf && b.0 > wf) || (a.1 > hf && b.1 > hf) { continue; }
        let Some((a, b)) = clip(a, b, wf, hf) else { continue };
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        let steps = dx.abs().max(dy.abs()).ceil().max(1.0) as usize;
        let (sx, sy) = (dx / steps as f32, dy / steps as f32);
        let wide = opt.thick && (e.seam || chosen);
        let (mut x, mut y) = a;
        for _ in 0..=steps {
            // The nearest pixel.
            let (px, py) = (((x + 0.5) as usize).min(w - 1), ((y + 0.5) as usize).min(h - 1));
            let i = py * w + px;
            if chosen { planes.picked[i] = 255; }
            else if e.seam { planes.seam[i] = 255; }
            else { planes.inner[i] = planes.inner[i].saturating_add(INNER_STEP); }
            if wide {
                let plane = if chosen { &mut planes.picked } else { &mut planes.seam };
                if px + 1 < w { plane[i + 1] = 255; }
                if py + 1 < h { plane[i + w] = 255; }
            }
            x += sx;
            y += sy;
        }
    }
}

/// Colours of the wire layer, as RGB.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Colours { pub inner: [u8; 3], pub seam: [u8; 3], pub picked: [u8; 3] }

/// Draw the edges into an RGBA buffer, transparent where there is no line.
pub fn raster(edges: &[Edge], view: &View, opt: &Options, colours: &Colours) -> Vec<u8> {
    let n = view.w * view.h;
    if n == 0 { return vec![]; }
    // Enough edges to be worth it: split them over the cores.
    let threads = if edges.len() < 50_000 { 1 } else { std::thread::available_parallelism().map(|c| c.get()).unwrap_or(4).clamp(1, 12) };
    let mut planes = Planes::new(n);
    if threads == 1 {
        draw(edges, view, opt, &mut planes);
    } else {
        let chunk = edges.len().div_ceil(threads);
        let parts: Vec<Planes> = std::thread::scope(|scope| {
            let handles: Vec<_> = edges.chunks(chunk).map(|part| scope.spawn(move || {
                let mut p = Planes::new(n);
                draw(part, view, opt, &mut p);
                p
            })).collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        for p in parts {
            for (o, i) in planes.inner.iter_mut().zip(&p.inner) { *o = o.saturating_add(*i); }
            for (o, i) in planes.seam.iter_mut().zip(&p.seam) { *o = (*o).max(*i); }
            for (o, i) in planes.picked.iter_mut().zip(&p.picked) { *o = (*o).max(*i); }
        }
    }
    let mut out = vec![0u8; n * 4];
    for i in 0..n {
        let (rgb, alpha) = if planes.picked[i] > 0 { (colours.picked, 255) }
            else if planes.seam[i] > 0 { (colours.seam, 255) }
            else if planes.inner[i] > 0 { (colours.inner, planes.inner[i].max(70)) }
            else { continue };
        out[i * 4..i * 4 + 3].copy_from_slice(&rgb);
        out[i * 4 + 3] = alpha;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLOURS: Colours = Colours { inner: [1, 1, 1], seam: [2, 2, 2], picked: [3, 3, 3] };
    fn view() -> View { View { w: 100, h: 100, scale: 100.0, ox: 0.0, oy: 99.0 } }
    fn lit(buf: &[u8]) -> usize { buf.chunks_exact(4).filter(|p| p[3] > 0).count() }
    fn at(buf: &[u8], x: usize, y: usize) -> [u8; 4] { let i = (y * 100 + x) * 4; [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]] }

    #[test]
    fn a_line_lights_the_pixels_it_crosses() {
        let e = [Edge { a: [0.1, 0.5], b: [0.9, 0.5], seam: true, island: 0 }];
        let buf = raster(&e, &view(), &Options::default(), &COLOURS);
        assert_eq!(buf.len(), 100 * 100 * 4);
        // v = 0.5 is the row 49 pixels down from the top.
        assert_eq!(at(&buf, 50, 49), [2, 2, 2, 255]);
        assert_eq!(at(&buf, 5, 49)[3], 0);
        assert!(lit(&buf) >= 80 && lit(&buf) <= 82, "{}", lit(&buf));
    }

    #[test]
    fn borders_only_drops_inner_edges_but_keeps_the_selection() {
        let e = [
            Edge { a: [0.1, 0.2], b: [0.9, 0.2], seam: false, island: 0 },
            Edge { a: [0.1, 0.4], b: [0.9, 0.4], seam: true, island: 0 },
            Edge { a: [0.1, 0.6], b: [0.9, 0.6], seam: false, island: 7 },
        ];
        let all = raster(&e, &view(), &Options::default(), &COLOURS);
        assert_eq!(at(&all, 50, 79)[..3], [1, 1, 1]);
        let borders = raster(&e, &view(), &Options { borders_only: true, selected: Some(7), thick: false }, &COLOURS);
        assert_eq!(at(&borders, 50, 79)[3], 0);
        assert_eq!(at(&borders, 50, 59), [2, 2, 2, 255]);
        assert_eq!(at(&borders, 50, 39), [3, 3, 3, 255]);
    }

    #[test]
    fn lines_off_the_canvas_cost_nothing_and_long_ones_are_clipped() {
        let off = [Edge { a: [2.0, 2.0], b: [3.0, 3.0], seam: true, island: 0 }];
        assert_eq!(lit(&raster(&off, &view(), &Options::default(), &COLOURS)), 0);
        // Far longer than the canvas: clipped to it, not walked end to end.
        let long = [Edge { a: [-1.0e6, 0.5], b: [1.0e6, 0.5], seam: true, island: 0 }];
        assert_eq!(lit(&raster(&long, &view(), &Options::default(), &COLOURS)), 100);
    }

    #[test]
    fn a_million_edges_come_out_the_same_on_many_threads() {
        // A dense fan of short inner edges and a few seams.
        let edges: Vec<Edge> = (0..1_000_000).map(|i| {
            let (u, v) = ((i % 1000) as f32 / 1000.0, (i / 1000) as f32 / 1000.0);
            Edge { a: [u, v], b: [u + 0.001, v + 0.0007], seam: i % 97 == 0, island: (i % 50) as u32 }
        }).collect();
        let many = raster(&edges, &view(), &Options::default(), &COLOURS);
        // The same edges in one piece, on one thread.
        let mut planes = Planes::new(100 * 100);
        draw(&edges, &view(), &Options::default(), &mut planes);
        let seams_one = planes.seam.iter().filter(|s| **s > 0).count();
        let seams_many = many.chunks_exact(4).filter(|p| p[..3] == [2, 2, 2]).count();
        assert_eq!(seams_one, seams_many);
        assert_eq!(lit(&many), 100 * 100);
    }
}
