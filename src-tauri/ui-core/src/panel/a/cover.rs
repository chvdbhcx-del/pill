//! Cover gradient of the Home player card (`pickColors` / `rgb2hsl` / `covCache` of `tab-home.js`).
//!
//! The renderer decodes the cover to RGBA, calls [`downsample`] to the 24x24 sample the JS used, then [`pick_colors`].
//! Result: two CSS-style `hsl(..)` strings (a = dominant hue, b = a second hue at least 60 degrees away, else a darkened,
//! hue-shifted a) or `None` for grey / near-black / near-white covers (the card stays neutral).

use std::collections::HashMap;

pub const SAMPLE: usize = 24;

/// Two gradient stops, formatted as `hsl(h, s%, l%)`.
#[derive(Clone, Debug, PartialEq)]
pub struct CoverColors {
    pub a: String,
    pub b: String,
}

/// RGB (0..255, may be fractional averages) to (hue degrees, saturation, lightness).
pub fn rgb2hsl(r: f64, g: f64, b: f64) -> (f64, f64, f64) {
    let (r, g, b) = (r / 255.0, g / 255.0, b / 255.0);
    let mx = r.max(g).max(b);
    let mn = r.min(g).min(b);
    let l = (mx + mn) / 2.0;
    let c = mx - mn;
    if c == 0.0 {
        return (0.0, 0.0, l);
    }
    let s = c / (1.0 - (2.0 * l - 1.0).abs());
    let h = if mx == r {
        ((g - b) / c + 6.0) % 6.0
    } else if mx == g {
        (b - r) / c + 2.0
    } else {
        (r - g) / c + 4.0
    };
    (h * 60.0, s, l)
}

fn clamp(v: f64, lo: f64, hi: f64) -> f64 {
    v.max(lo).min(hi)
}

fn css((h, s, l): (f64, f64, f64)) -> String {
    format!("hsl({}, {}%, {}%)", h.round() as i64, (s * 100.0).round() as i64, (l * 100.0).round() as i64)
}

/// 12 hue buckets weighted by saturation and mid-lightness; near-black / near-white / grey / translucent pixels are ignored.
/// `rgba` is a tightly packed RGBA buffer (any size; the JS fed a 24x24 sample, see [`downsample`]).
pub fn pick_colors(rgba: &[u8]) -> Option<CoverColors> {
    #[derive(Clone, Copy, Default)]
    struct Bucket {
        w: f64,
        r: f64,
        g: f64,
        b: f64,
    }
    let mut buckets = [Bucket::default(); 12];
    for px in rgba.chunks_exact(4) {
        if px[3] < 200 {
            continue;
        }
        let (h, s, l) = rgb2hsl(px[0] as f64, px[1] as f64, px[2] as f64);
        if l < 0.12 || l > 0.9 || s < 0.18 {
            continue;
        }
        let w = s * (1.0 - (2.0 * l - 1.0).abs());
        let k = &mut buckets[((h / 30.0).floor() as usize).min(11)];
        k.w += w;
        k.r += px[0] as f64 * w;
        k.g += px[1] as f64 * w;
        k.b += px[2] as f64 * w;
    }
    let mut ia = 0;
    for (i, k) in buckets.iter().enumerate() {
        if k.w > buckets[ia].w {
            ia = i;
        }
    }
    if buckets[ia].w == 0.0 {
        return None;
    }
    let mut ib: Option<usize> = None;
    for (i, k) in buckets.iter().enumerate() {
        let dist = ((i + 12 - ia) % 12).min((ia + 12 - i) % 12);
        if dist >= 2 && k.w > buckets[ia].w * 0.15 && ib.map_or(true, |j| k.w > buckets[j].w) {
            ib = Some(i);
        }
    }
    let avg = |k: &Bucket| rgb2hsl(k.r / k.w, k.g / k.w, k.b / k.w);
    let (ha, sa, la) = avg(&buckets[ia]);
    let a = (ha, clamp(sa, 0.35, 0.85), clamp(la, 0.32, 0.6));
    let b = match ib {
        Some(j) => {
            let (h, s, l) = avg(&buckets[j]);
            (h, clamp(s, 0.3, 0.8), clamp(l, 0.22, 0.5))
        }
        None => ((ha + 25.0) % 360.0, a.1, a.2 * 0.65),
    };
    Some(CoverColors { a: css(a), b: css(b) })
}

/// Box-average an RGBA image down to `SAMPLE` x `SAMPLE` (the JS drew the cover onto a 24x24 canvas).
pub fn downsample(rgba: &[u8], w: usize, h: usize) -> Vec<u8> {
    let n = SAMPLE;
    let mut out = vec![0u8; n * n * 4];
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return out;
    }
    for oy in 0..n {
        let (y0, y1) = (oy * h / n, ((oy + 1) * h / n).max(oy * h / n + 1).min(h));
        for ox in 0..n {
            let (x0, x1) = (ox * w / n, ((ox + 1) * w / n).max(ox * w / n + 1).min(w));
            let mut acc = [0u64; 4];
            let mut cnt = 0u64;
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = (y * w + x) * 4;
                    for c in 0..4 {
                        acc[c] += rgba[i + c] as u64;
                    }
                    cnt += 1;
                }
            }
            let o = (oy * n + ox) * 4;
            for c in 0..4 {
                out[o + c] = (acc[c] / cnt.max(1)) as u8;
            }
        }
    }
    out
}

/// `covCache`: key -> colors (None = grey / failed, so the work is not repeated). Cleared when it passes 40 entries.
#[derive(Default)]
pub struct CoverCache {
    map: HashMap<String, Option<CoverColors>>,
}

impl CoverCache {
    pub fn get(&self, key: &str) -> Option<&Option<CoverColors>> {
        self.map.get(key)
    }
    pub fn put(&mut self, key: String, v: Option<CoverColors>) {
        if self.map.len() > 40 {
            self.map.clear();
        }
        self.map.insert(key, v);
    }
    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(r: u8, g: u8, b: u8, n: usize) -> Vec<u8> {
        (0..n).flat_map(|_| [r, g, b, 255]).collect()
    }

    #[test]
    fn hsl_of_primaries() {
        let (h, s, l) = rgb2hsl(255.0, 0.0, 0.0);
        assert_eq!((h, s, l), (0.0, 1.0, 0.5));
        assert_eq!(rgb2hsl(0.0, 255.0, 0.0).0, 120.0);
        assert_eq!(rgb2hsl(0.0, 0.0, 255.0).0, 240.0);
        assert_eq!(rgb2hsl(128.0, 128.0, 128.0).1, 0.0);
    }

    #[test]
    fn grey_black_white_and_translucent_give_no_gradient() {
        assert_eq!(pick_colors(&solid(128, 128, 128, 576)), None);
        assert_eq!(pick_colors(&solid(5, 5, 5, 576)), None);
        assert_eq!(pick_colors(&solid(250, 250, 250, 576)), None);
        let translucent: Vec<u8> = (0..576).flat_map(|_| [255, 0, 0, 100]).collect();
        assert_eq!(pick_colors(&translucent), None);
        assert_eq!(pick_colors(&[]), None);
    }

    #[test]
    fn single_hue_darkens_and_shifts_for_b() {
        let c = pick_colors(&solid(255, 0, 0, 576)).unwrap();
        assert_eq!(c.a, "hsl(0, 85%, 50%)"); // saturation clamped to .85
        assert_eq!(c.b, "hsl(25, 85%, 33%)"); // 0 + 25 degrees, lightness * .65
    }

    #[test]
    fn two_distant_hues_use_the_second_bucket() {
        // two thirds red, one third blue: a = red, b = blue
        let mut px = solid(220, 30, 30, 384);
        px.extend(solid(30, 40, 220, 192));
        let c = pick_colors(&px).unwrap();
        assert!(c.a.starts_with("hsl(0,") || c.a.starts_with("hsl(359,"), "{}", c.a);
        assert!(c.b.starts_with("hsl(23"), "{}", c.b);
        // a second hue under 60 degrees away is not used: orange next to red falls back to the shifted red
        let mut near = solid(220, 30, 30, 384);
        near.extend(solid(220, 120, 30, 192));
        let c = pick_colors(&near).unwrap();
        assert!(c.b.ends_with("%)"), "{}", c.b);
        assert!(!c.b.starts_with("hsl(2"), "{}", c.b); // not a hue in the 20s..: orange is ~28 degrees, red ~0 -> dist 1 bucket
    }

    #[test]
    fn downsample_averages_blocks() {
        let mut img = Vec::new();
        for y in 0..48 {
            for _x in 0..48 {
                img.extend_from_slice(if y < 24 { &[200, 0, 0, 255] } else { &[0, 0, 200, 255] });
            }
        }
        let s = downsample(&img, 48, 48);
        assert_eq!(s.len(), 24 * 24 * 4);
        assert_eq!(&s[0..4], &[200, 0, 0, 255]);
        assert_eq!(&s[(23 * 24) * 4..(23 * 24) * 4 + 4], &[0, 0, 200, 255]);
        assert_eq!(downsample(&[], 0, 0).len(), 24 * 24 * 4);
    }

    #[test]
    fn cache_clears_past_forty() {
        let mut c = CoverCache::default();
        for i in 0..41 {
            c.put(format!("{i}:1"), None);
        }
        assert_eq!(c.len(), 41);
        c.put("x".into(), None);
        assert_eq!(c.len(), 1);
        assert_eq!(c.get("x"), Some(&None));
        assert!(c.get("0:1").is_none());
    }
}
