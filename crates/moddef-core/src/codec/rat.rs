//! Exact rational arithmetic over i128 (spec §10). A 64-bit raw register
//! value times an i64/i64 rational fits i128 with headroom; gcd
//! normalization after each operation keeps magnitudes small. `no_std`.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rat {
    n: i128,
    d: i128, // > 0
}

const fn gcd(mut a: i128, mut b: i128) -> i128 {
    if a < 0 {
        a = -a;
    }
    if b < 0 {
        b = -b;
    }
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    if a == 0 {
        1
    } else {
        a
    }
}

// Plain methods rather than ops traits: every operation is used in call
// chains where the by-value Copy semantics read clearer than `impl Mul`.
#[allow(clippy::should_implement_trait)]
impl Rat {
    pub fn new(n: i128, d: i128) -> Rat {
        let (mut n, mut d) = if d < 0 { (-n, -d) } else { (n, d) };
        let g = gcd(n, d);
        n /= g;
        d /= g;
        Rat { n, d }
    }

    pub fn int(v: i64) -> Rat {
        Rat { n: v as i128, d: 1 }
    }

    pub fn from_u64(v: u64) -> Rat {
        Rat { n: v as i128, d: 1 }
    }

    pub fn mul(self, o: Rat) -> Rat {
        // Cross-reduce before multiplying to avoid overflow.
        let g1 = gcd(self.n, o.d);
        let g2 = gcd(o.n, self.d);
        Rat::new((self.n / g1) * (o.n / g2), (self.d / g2) * (o.d / g1))
    }

    pub fn div(self, o: Rat) -> Rat {
        self.mul(Rat::new(o.d, o.n))
    }

    pub fn add(self, o: Rat) -> Rat {
        Rat::new(self.n * o.d + o.n * self.d, self.d * o.d)
    }

    pub fn sub(self, o: Rat) -> Rat {
        Rat::new(self.n * o.d - o.n * self.d, self.d * o.d)
    }

    /// 10^exp as an exact rational (|exp| <= 38 fits i128; clamp beyond).
    pub fn pow10(exp: i64) -> Rat {
        let e = exp.unsigned_abs().min(38);
        let mut p: i128 = 1;
        let mut i = 0;
        while i < e {
            p *= 10;
            i += 1;
        }
        if exp >= 0 {
            Rat { n: p, d: 1 }
        } else {
            Rat { n: 1, d: p }
        }
    }

    /// Round to nearest integer, half away from zero (matches Go ratRound).
    pub fn round(self) -> i64 {
        let half = self.d / 2;
        let n = if self.n >= 0 {
            self.n + half
        } else {
            self.n - half
        };
        (n / self.d) as i64
    }

    pub fn to_f64(self) -> f64 {
        // Split to keep precision for large numerators.
        let q = self.n / self.d;
        let r = self.n % self.d;
        q as f64 + (r as f64) / (self.d as f64)
    }

    /// Parse an f64 into an exact rational via its shortest decimal display
    /// (keeps typical engineering values like 230.1 exact). Fallback: rounded.
    pub fn from_f64(v: f64) -> Rat {
        if !v.is_finite() {
            return Rat::int(0);
        }
        if v == (v as i64) as f64 {
            return Rat::int(v as i64);
        }
        // Scale by powers of 10 until integral (max 12 fractional digits).
        // f64::trunc/round live in std, not core; saturating float->int casts
        // give no_std-safe equivalents within our value range.
        let mut d: i128 = 1;
        let mut x = v;
        for _ in 0..12 {
            if x == (x as i128) as f64 {
                break;
            }
            x *= 10.0;
            d *= 10;
        }
        let n = if x >= 0.0 {
            (x + 0.5) as i128
        } else {
            (x - 0.5) as i128
        };
        Rat::new(n, d)
    }
}
