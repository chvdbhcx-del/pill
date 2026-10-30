//! CSS easing functions as exported by the browser: keywords (l e i o io), cubic-bezier(), steps(), linear(...).
//! Our own small solver (glasskit::motion has springs, not CSS curves).

#[derive(Clone, Debug, PartialEq)]
pub enum Ease {
    Linear,
    Bezier([f32; 4]),
    /// jumps (N intervals), `start` jumps at the start of each interval, `none` / `both` per the CSS spec
    Steps { n: u32, kind: StepKind },
    /// linear(): piecewise-linear through (progress, output) points, already sorted by progress
    Points(Vec<(f32, f32)>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepKind {
    End,
    Start,
    None,
    Both,
}

const EASE: [f32; 4] = [0.25, 0.1, 0.25, 1.0];
const EASE_IN: [f32; 4] = [0.42, 0.0, 1.0, 1.0];
const EASE_OUT: [f32; 4] = [0.0, 0.0, 0.58, 1.0];
const EASE_IN_OUT: [f32; 4] = [0.42, 0.0, 0.58, 1.0];

impl Ease {
    pub fn parse(s: &str) -> Ease {
        let s = s.trim();
        match s {
            "l" | "linear" => return Ease::Linear,
            "e" | "ease" => return Ease::Bezier(EASE),
            "i" | "ease-in" => return Ease::Bezier(EASE_IN),
            "o" | "ease-out" => return Ease::Bezier(EASE_OUT),
            "io" | "ease-in-out" => return Ease::Bezier(EASE_IN_OUT),
            _ => {}
        }
        if let Some(a) = s.strip_prefix("cubic-bezier(").and_then(|r| r.strip_suffix(')')) {
            let v: Vec<f32> = a.split(',').filter_map(|x| x.trim().parse().ok()).collect();
            if v.len() == 4 {
                return Ease::Bezier([v[0], v[1], v[2], v[3]]);
            }
        }
        if let Some(a) = s.strip_prefix("steps(").and_then(|r| r.strip_suffix(')')) {
            let mut it = a.split(',').map(str::trim);
            let n = it.next().and_then(|x| x.parse().ok()).unwrap_or(1u32).max(1);
            let kind = match it.next() {
                Some("start") | Some("jump-start") => StepKind::Start,
                Some("jump-none") => StepKind::None,
                Some("jump-both") => StepKind::Both,
                _ => StepKind::End,
            };
            return Ease::Steps { n, kind };
        }
        if let Some(a) = s.strip_prefix("linear(").and_then(|r| r.strip_suffix(')')) {
            let mut pts: Vec<(f32, f32)> = Vec::new();
            for part in a.split(',') {
                let mut it = part.split_whitespace();
                let (Some(v), p) = (it.next().and_then(|x| x.parse::<f32>().ok()), it.next()) else { continue };
                let pos = p.and_then(|x| x.trim_end_matches('%').parse::<f32>().ok()).map(|x| x / 100.0);
                pts.push((pos.unwrap_or(f32::NAN), v));
            }
            // fill missing positions: first 0, last 1, others evenly between known neighbours
            if let Some(f) = pts.first_mut() {
                if f.0.is_nan() {
                    f.0 = 0.0;
                }
            }
            if let Some(l) = pts.last_mut() {
                if l.0.is_nan() {
                    l.0 = 1.0;
                }
            }
            let mut i = 0;
            while i < pts.len() {
                if pts[i].0.is_nan() {
                    let mut j = i;
                    while pts[j].0.is_nan() {
                        j += 1;
                    }
                    let (a0, a1) = (pts[i - 1].0, pts[j].0);
                    for k in i..j {
                        pts[k].0 = a0 + (a1 - a0) * (k - i + 1) as f32 / (j - i + 1) as f32;
                    }
                    i = j;
                }
                i += 1;
            }
            if pts.len() >= 2 {
                return Ease::Points(pts);
            }
        }
        Ease::Linear
    }

    /// Output progress for input progress `x` (0..1).
    pub fn at(&self, x: f32) -> f32 {
        match self {
            Ease::Linear => x,
            Ease::Bezier(b) => bezier(*b, x),
            Ease::Steps { n, kind } => {
                let nf = *n as f32;
                let mut step = (x * nf).floor();
                if matches!(kind, StepKind::Start | StepKind::Both) {
                    step += 1.0;
                }
                let jumps = match kind {
                    StepKind::None => nf - 1.0,
                    StepKind::Both => nf + 1.0,
                    _ => nf,
                };
                if x >= 1.0 {
                    return 1.0;
                }
                (step / jumps.max(1.0)).clamp(0.0, 1.0)
            }
            Ease::Points(p) => {
                if x <= p[0].0 {
                    return p[0].1;
                }
                for w in p.windows(2) {
                    if x <= w[1].0 {
                        let span = w[1].0 - w[0].0;
                        return if span <= 0.0 { w[1].1 } else { w[0].1 + (w[1].1 - w[0].1) * (x - w[0].0) / span };
                    }
                }
                p[p.len() - 1].1
            }
        }
    }
}

/// y(x) of a CSS cubic-bezier with control points (x1,y1),(x2,y2): Newton on x(t), bisection fallback.
pub fn bezier(b: [f32; 4], x: f32) -> f32 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }
    let [x1, y1, x2, y2] = b.map(|v| v as f64);
    let cx = 3.0 * x1;
    let bx = 3.0 * (x2 - x1) - cx;
    let ax = 1.0 - cx - bx;
    let cy = 3.0 * y1;
    let by = 3.0 * (y2 - y1) - cy;
    let ay = 1.0 - cy - by;
    let sx = |t: f64| ((ax * t + bx) * t + cx) * t;
    let dx = |t: f64| (3.0 * ax * t + 2.0 * bx) * t + cx;
    let x = x as f64;
    let mut t = x;
    for _ in 0..8 {
        let e = sx(t) - x;
        if e.abs() < 1e-6 {
            return (((ay * t + by) * t + cy) * t) as f32;
        }
        let d = dx(t);
        if d.abs() < 1e-6 {
            break;
        }
        t -= e / d;
    }
    let (mut lo, mut hi) = (0.0f64, 1.0f64);
    t = x;
    for _ in 0..40 {
        let e = sx(t);
        if (e - x).abs() < 1e-6 {
            break;
        }
        if e < x {
            lo = t;
        } else {
            hi = t;
        }
        t = (lo + hi) / 2.0;
    }
    (((ay * t + by) * t + cy) * t) as f32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_match_css() {
        // reference values from the CSS specification curves
        let ease = Ease::parse("e");
        assert!((ease.at(0.5) - 0.8024).abs() < 1e-3, "{}", ease.at(0.5));
        let eo = Ease::parse("o");
        assert!((eo.at(0.5) - 0.6846).abs() < 2e-3, "{}", eo.at(0.5)); // solved by hand: t = 0.6257 -> y = 0.6846
        let eio = Ease::parse("io");
        assert!((eio.at(0.5) - 0.5).abs() < 1e-3);
        assert_eq!(eio.at(0.0), 0.0);
        assert_eq!(eio.at(1.0), 1.0);
    }

    #[test]
    fn overshoot_curve() {
        let e = Ease::parse("cubic-bezier(0.34,1.56,0.64,1)");
        let peak = (1..100).map(|i| e.at(i as f32 / 100.0)).fold(0.0f32, f32::max);
        assert!(peak > 1.05, "spring-like curve must overshoot, peak {peak}");
        assert!((e.at(1.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn steps_variants() {
        let s = Ease::parse("steps(4)");
        assert_eq!(s.at(0.0), 0.0);
        assert_eq!(s.at(0.26), 0.25);
        assert_eq!(s.at(0.99), 0.75);
        assert_eq!(s.at(1.0), 1.0);
        let one = Ease::parse("steps(1)");
        assert_eq!((one.at(0.0), one.at(0.99)), (0.0, 0.0));
        let jn = Ease::parse("steps(2,jump-none)");
        assert_eq!((jn.at(0.1), jn.at(0.6), jn.at(1.0)), (0.0, 1.0, 1.0));
        let st = Ease::parse("steps(2,start)");
        assert_eq!((st.at(0.0), st.at(0.6)), (0.5, 1.0));
    }

    #[test]
    fn linear_points() {
        let e = Ease::parse("linear(0 0%,0.5 25%,1.1 50%,1 100%)");
        assert!((e.at(0.125) - 0.25).abs() < 1e-5);
        assert!((e.at(0.5) - 1.1).abs() < 1e-5);
        assert!((e.at(1.0) - 1.0).abs() < 1e-5);
        let spring = Ease::parse("linear(0 0%, 0.044 3.5%, 1.094 42.8%, 1 100%)");
        assert!(spring.at(0.428) > 1.09);
    }

    #[test]
    fn unknown_is_linear() {
        assert_eq!(Ease::parse("whatever"), Ease::Linear);
    }
}
