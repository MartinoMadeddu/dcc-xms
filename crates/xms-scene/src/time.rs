//! Time samples, with USD's evaluation rules: linear interpolation between
//! samples, values held before the first and after the last.

/// Interpolation between two values (`t` in 0..1).
pub trait Lerp: Sized {
    fn lerp(&self, b: &Self, t: f64) -> Self;
}

impl Lerp for f32 {
    fn lerp(&self, b: &f32, t: f64) -> f32 {
        self + (b - self) * t as f32
    }
}

impl Lerp for f64 {
    fn lerp(&self, b: &f64, t: f64) -> f64 {
        self + (b - self) * t
    }
}

impl Lerp for bool {
    /// Booleans don't interpolate: held until the next sample.
    fn lerp(&self, b: &bool, t: f64) -> bool {
        if t < 1.0 { *self } else { *b }
    }
}

impl<const N: usize> Lerp for [f32; N] {
    fn lerp(&self, b: &Self, t: f64) -> Self {
        std::array::from_fn(|i| self[i] + (b[i] - self[i]) * t as f32)
    }
}

impl<const N: usize> Lerp for [f64; N] {
    fn lerp(&self, b: &Self, t: f64) -> Self {
        std::array::from_fn(|i| self[i] + (b[i] - self[i]) * t)
    }
}

impl<T: Lerp + Clone> Lerp for Vec<T> {
    /// Element-wise; arrays of different lengths can't interpolate (USD holds
    /// the earlier sample then).
    fn lerp(&self, b: &Self, t: f64) -> Self {
        if self.len() != b.len() {
            return if t < 1.0 { self.clone() } else { b.clone() };
        }
        self.iter().zip(b).map(|(x, y)| x.lerp(y, t)).collect()
    }
}

/// A value with zero or more time samples. One sample (or a default) = constant.
#[derive(Clone, Debug, PartialEq)]
pub struct Sampled<T> {
    /// (time code, value), sorted by time, times unique
    samples: Vec<(f64, T)>,
}

impl<T> Default for Sampled<T> {
    fn default() -> Self {
        Sampled { samples: Vec::new() }
    }
}

impl<T: Clone> Sampled<T> {
    pub fn constant(v: T) -> Self {
        Sampled { samples: vec![(0.0, v)] }
    }

    /// From (time, value) pairs in any order; later duplicates of a time win.
    pub fn from_samples(mut samples: Vec<(f64, T)>) -> Self {
        samples.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut out: Vec<(f64, T)> = Vec::with_capacity(samples.len());
        for (t, v) in samples {
            match out.last_mut() {
                Some(last) if last.0 == t => last.1 = v,
                _ => out.push((t, v)),
            }
        }
        Sampled { samples: out }
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// More than one sample: the value changes over time.
    pub fn is_animated(&self) -> bool {
        self.samples.len() > 1
    }

    pub fn samples(&self) -> &[(f64, T)] {
        &self.samples
    }

    pub fn times(&self) -> impl Iterator<Item = f64> + '_ {
        self.samples.iter().map(|s| s.0)
    }

    /// Any value (the first sample), for data that doesn't need time.
    pub fn first(&self) -> Option<&T> {
        self.samples.first().map(|s| &s.1)
    }

    /// The samples that bracket `t`: (index a, index b, weight of b).
    fn bracket(&self, t: f64) -> Option<(usize, usize, f64)> {
        let n = self.samples.len();
        if n == 0 {
            return None;
        }
        if n == 1 || t <= self.samples[0].0 {
            return Some((0, 0, 0.0));
        }
        if t >= self.samples[n - 1].0 {
            return Some((n - 1, n - 1, 0.0));
        }
        let b = self.samples.partition_point(|s| s.0 <= t);
        let a = b - 1;
        let (ta, tb) = (self.samples[a].0, self.samples[b].0);
        Some((a, b, (t - ta) / (tb - ta)))
    }

    /// The samples needed to cover the interval [open, close]: every sample inside
    /// it, plus the bracketing samples on either side (for motion blur).
    pub fn within(&self, open: f64, close: f64) -> &[(f64, T)] {
        let n = self.samples.len();
        if n <= 1 {
            return &self.samples;
        }
        let lo = self.samples.partition_point(|s| s.0 <= open).saturating_sub(1);
        let hi = (self.samples.partition_point(|s| s.0 < close) + 1).min(n);
        &self.samples[lo..hi.max(lo + 1)]
    }
}

impl<T: Clone + Lerp> Sampled<T> {
    /// Value at time `t`: linear between samples, held outside their range.
    pub fn at(&self, t: f64) -> Option<T> {
        let (a, b, w) = self.bracket(t)?;
        Some(if a == b || w <= 0.0 { self.samples[a].1.clone() } else { self.samples[a].1.lerp(&self.samples[b].1, w) })
    }

    /// [`Sampled::at`], consuming the samples: a value at a sample (always, for static
    /// attributes) is moved rather than cloned; between samples it's interpolated.
    pub fn into_at(mut self, t: f64) -> Option<T> {
        let (a, b, w) = self.bracket(t)?;
        Some(if a == b || w <= 0.0 { self.samples.swap_remove(a).1 } else { self.samples[a].1.lerp(&self.samples[b].1, w) })
    }
}

/// Scene time: the current time code, and the shutter interval for motion blur
/// (as offsets from it, in frames: USD's `shutter:open` / `shutter:close`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneTime {
    pub time: f64,
    pub shutter_open: f64,
    pub shutter_close: f64,
}

impl Default for SceneTime {
    fn default() -> Self {
        SceneTime { time: 0.0, shutter_open: 0.0, shutter_close: 0.0 }
    }
}

impl SceneTime {
    /// Absolute time codes of the shutter interval.
    pub fn interval(&self) -> (f64, f64) {
        (self.time + self.shutter_open, self.time + self.shutter_close)
    }

    pub fn has_motion_blur(&self) -> bool {
        self.shutter_close > self.shutter_open
    }
}
