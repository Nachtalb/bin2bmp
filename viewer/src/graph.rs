//! 3D byte graphs: counting, placing and drawing the points. No GUI types in here.

use std::collections::HashMap;

pub const SLICES: u32 = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Every three neighbouring bytes are one point in the cube.
    Tri,
    /// The flat digraph, each pair raised by how often it occurs.
    Height,
    /// The digraph stacked by position in the file.
    Layers,
}

impl Mode {
    pub const ALL: [Mode; 3] = [Mode::Tri, Mode::Height, Mode::Layers];

    pub fn label(self) -> &'static str {
        match self {
            Mode::Tri => "Trigraph",
            Mode::Height => "Digraph + height",
            Mode::Layers => "Digraph over file",
        }
    }

    pub fn about(self) -> &'static str {
        match self {
            Mode::Tri => "Every three neighbouring bytes are one point in the cube.",
            Mode::Height => "The flat digraph, with every pair raised by how often it occurs.",
            Mode::Layers => {
                "The digraph stacked by position: bottom is the start of the file, top is \
                 the end. Sections of the file show up as layers."
            }
        }
    }

    /// What the x, y and z axes mean.
    pub fn axes(self) -> [&'static str; 3] {
        match self {
            Mode::Tri => ["byte i", "byte i+1", "byte i+2"],
            Mode::Height => ["byte i", "how often", "byte i+1"],
            Mode::Layers => ["byte i", "position in file", "byte i+1"],
        }
    }
}

pub struct Tally {
    /// key → (count, where in the file it first occurs as 0..=65535)
    pub counts: HashMap<u32, (u32, u16)>,
    /// Big inputs are sampled: only every `stride`-th position is counted.
    pub stride: usize,
}

pub fn tally(bytes: &[u8], mode: Mode, max_samples: usize) -> Tally {
    let starts = bytes
        .len()
        .saturating_sub(if mode == Mode::Tri { 2 } else { 1 });
    let stride = starts.div_ceil(max_samples).max(1);
    let mut counts = HashMap::new();
    for i in (0..starts).step_by(stride) {
        let mut key = (bytes[i] as u32) << 8 | bytes[i + 1] as u32;
        let part = |n: u64| (i as u64 * n / starts as u64) as u32;
        match mode {
            Mode::Tri => key = key << 8 | bytes[i + 2] as u32,
            Mode::Height => {}
            Mode::Layers => key |= part(SLICES as u64) << 16,
        }
        counts.entry(key).or_insert((0, part(65536) as u16)).0 += 1;
    }
    Tally { counts, stride }
}

/// Where a key sits in the unit cube; `level` is its log-scaled count, 0..=1.
pub fn place(mode: Mode, key: u32, level: f32) -> [f32; 3] {
    let [hi, mid, lo] = [key >> 16 & 255, key >> 8 & 255, key & 255].map(|v| v as f32 / 255.0);
    match mode {
        Mode::Tri => [hi, mid, lo],
        Mode::Height => [mid, level, lo],
        Mode::Layers => [mid, hi * 255.0 / (SLICES - 1) as f32, lo],
    }
}

/// HSL with full saturation.
pub fn hue(h: f32, light: f32) -> [u8; 3] {
    [0.0, 8.0, 4.0].map(|n| {
        let k = (n + h * 12.0) % 12.0;
        let v = light - light.min(1.0 - light) * (k - 3.0).min(9.0 - k).clamp(-1.0, 1.0);
        (v * 255.0) as u8
    })
}

pub struct Cloud {
    /// Position (each axis -0.5..=0.5) and colour of every point.
    pub points: Vec<([f32; 3], [u8; 3])>,
    pub unique: usize,
    pub max: u32,
}

/// Points for a tally; more than `max_points` keys are thinned to a random subset
/// so density still reads right.
pub fn cloud(t: &Tally, mode: Mode, by_position: bool, min_count: u32, max_points: usize) -> Cloud {
    let mut keys: Vec<_> = t.counts.iter().filter(|(_, v)| v.0 >= min_count).collect();
    let unique = keys.len();
    if unique > max_points {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        for i in 0..max_points {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            keys.swap(i, i + (seed % (unique - i) as u64) as usize);
        }
        keys.truncate(max_points);
    }
    let max = keys.iter().map(|(_, v)| v.0).max().unwrap_or(1);
    let points = keys
        .into_iter()
        .map(|(&key, &(count, first))| {
            let level = (count as f32 + 1.0).ln() / (max as f32 + 1.0).ln();
            let colour = if by_position {
                hue(0.8 * first as f32 / 65535.0, 0.55) // red (start) → violet (end)
            } else {
                hue(0.66 - 0.66 * level, 0.25 + 0.4 * level) // blue (rare) → red (common)
            };
            (place(mode, key, level).map(|v| v - 0.5), colour)
        })
        .collect();
    Cloud {
        points,
        unique,
        max,
    }
}

/// Orbiting camera that looks at the centre of the cube.
#[derive(Clone, Copy, PartialEq)]
pub struct Camera {
    pub yaw: f32,
    pub pitch: f32,
    pub distance: f32,
}

impl Camera {
    fn project(&self, [x, y, z]: [f32; 3], [w, h]: [usize; 2]) -> Option<(f32, f32)> {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        let (x, z) = (x * cy + z * sy, z * cy - x * sy);
        let (y, z) = (y * cp - z * sp, y * sp + z * cp);
        let depth = self.distance - z;
        let focal = h as f32 / 2.0 / 25f32.to_radians().tan();
        (depth > 0.05).then(|| {
            (
                w as f32 / 2.0 + focal * x / depth,
                h as f32 / 2.0 - focal * y / depth,
            )
        })
    }
}

pub const AXIS_COLOURS: [[u8; 3]; 3] = [[255, 107, 107], [107, 219, 122], [107, 168, 255]];
const BACKGROUND: [u8; 3] = [11, 11, 14];

/// Draws the cube, its axes and the cloud into an RGB buffer of `size`.
pub fn draw(cloud: &Cloud, camera: &Camera, size: [usize; 2], point: usize) -> Vec<[u8; 3]> {
    let [w, h] = size;
    let mut px = vec![BACKGROUND; w * h];
    let mut dot = |at: Option<(f32, f32)>, colour: [u8; 3], size: usize, add: bool| {
        let Some((x, y)) = at else { return };
        let (x, y) = (
            x as isize - size as isize / 2,
            y as isize - size as isize / 2,
        );
        for (x, y) in (0..size * size).map(|i| (x + (i % size) as isize, y + (i / size) as isize)) {
            if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
                let p = &mut px[y as usize * w + x as usize];
                *p = if add {
                    [0, 1, 2].map(|c| p[c].saturating_add((colour[c] as u16 * 7 / 10) as u8))
                } else {
                    colour
                };
            }
        }
    };

    let corner = |i: usize| [0, 1, 2].map(|axis| if i >> axis & 1 == 1 { 0.5 } else { -0.5 });
    for a in 0..8usize {
        for (axis, axis_colour) in AXIS_COLOURS.into_iter().enumerate() {
            let b = a | 1 << axis;
            if a == b {
                continue;
            }
            // The three edges leaving the all-zero corner are the axes.
            let colour = if a == 0 { axis_colour } else { [68, 68, 85] };
            let (from, to) = (corner(a), corner(b));
            for step in 0..=600 {
                let t = step as f32 / 600.0;
                let at = [0, 1, 2].map(|c| from[c] + (to[c] - from[c]) * t);
                dot(camera.project(at, size), colour, 1, false);
            }
        }
    }
    for &(at, colour) in &cloud.points {
        dot(camera.project(at, size), colour, point, true);
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    fn counts(t: &Tally) -> Vec<(u32, u32)> {
        let mut v: Vec<_> = t.counts.iter().map(|(&k, &(n, _))| (k, n)).collect();
        v.sort();
        v
    }

    #[test]
    fn trigraph_counts_every_triple() {
        let t = tally(&[1, 2, 3, 1, 2, 3], Mode::Tri, 1 << 20);
        assert_eq!(counts(&t), [(0x010203, 2), (0x020301, 1), (0x030102, 1)]);
        assert_eq!(
            place(Mode::Tri, 0x010203, 0.0),
            [1.0 / 255.0, 2.0 / 255.0, 3.0 / 255.0]
        );
    }

    #[test]
    fn digraph_height_raises_pairs_by_count() {
        let t = tally(&[9, 7, 9, 7, 5], Mode::Height, 1 << 20);
        assert_eq!(counts(&t), [(0x0705, 1), (0x0709, 1), (0x0907, 2)]);
        assert_eq!(
            place(Mode::Height, 0x0907, 0.5),
            [9.0 / 255.0, 0.5, 7.0 / 255.0]
        );
    }

    #[test]
    fn layers_run_from_the_bottom_slice_to_the_top_one() {
        let ramp: Vec<u8> = (0..1025u32).map(|i| i as u8).collect();
        let t = tally(&ramp, Mode::Layers, 1 << 20);
        let slices: Vec<u32> = counts(&t).iter().map(|(k, _)| k >> 16).collect();
        assert_eq!((slices[0], slices[slices.len() - 1]), (0, 255));
        assert_eq!(place(Mode::Layers, 255 << 16 | 0x0102, 0.0)[1], 1.0);
        assert_eq!(
            t.counts[&(255 << 16 | 0xFEFF)].1,
            65408,
            "first seen near the end"
        );
    }

    #[test]
    fn big_inputs_are_sampled_and_tiny_ones_are_empty() {
        assert_eq!(tally(&[0; 1000], Mode::Tri, 100).stride, 10);
        assert!(tally(&[1, 2], Mode::Tri, 100).counts.is_empty());
        assert!(tally(&[], Mode::Height, 100).counts.is_empty());
    }

    #[test]
    fn cloud_hides_rare_keys_and_thins_crowds() {
        let t = tally(&[9, 7, 9, 7, 5], Mode::Height, 1 << 20);
        let c = cloud(&t, Mode::Height, false, 2, 10);
        assert_eq!((c.unique, c.max, c.points.len()), (1, 2, 1));
        assert_eq!(c.points[0].0, [9.0 / 255.0 - 0.5, 0.5, 7.0 / 255.0 - 0.5]);

        let ramp: Vec<u8> = (0..1025u32).map(|i| i as u8).collect();
        let c = cloud(&tally(&ramp, Mode::Tri, 1 << 20), Mode::Tri, true, 1, 50);
        assert_eq!((c.unique, c.points.len()), (256, 50));
        let mut seen: Vec<_> = c.points.iter().map(|p| p.0.map(f32::to_bits)).collect();
        seen.sort();
        seen.dedup();
        assert_eq!(seen.len(), 50, "thinning must not repeat a point");
    }

    #[test]
    fn camera_centres_the_cube_and_shrinks_what_is_further_away() {
        let camera = Camera {
            yaw: 0.0,
            pitch: 0.0,
            distance: 3.0,
        };
        assert_eq!(camera.project([0.0; 3], [200, 100]), Some((100.0, 50.0)));
        let near = camera.project([0.5, 0.0, 0.5], [200, 100]).unwrap().0;
        let far = camera.project([0.5, 0.0, -0.5], [200, 100]).unwrap().0;
        assert!(near > far && far > 100.0);

        let dotted = Cloud {
            points: vec![([0.0; 3], [200, 100, 0])],
            unique: 1,
            max: 1,
        };
        let px = draw(&dotted, &camera, [200, 100], 1);
        assert_eq!(px[50 * 200 + 100], [11 + 140, 11 + 70, 14]);
    }

    #[test]
    fn hue_runs_red_green_blue() {
        assert_eq!(
            [hue(0.0, 0.5), hue(1.0 / 3.0, 0.5), hue(2.0 / 3.0, 0.5)],
            [[255, 0, 0], [0, 255, 0], [0, 0, 255]]
        );
    }
}
