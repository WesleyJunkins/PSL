//! Small helpers with no better home: semver, SHA-256, time, paths.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A semantic version `major.minor.patch`. Pre-release and build tags are kept as text and ignored in ordering.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl Version {
    pub fn parse(s: &str) -> Option<Version> {
        let core = s.split(['-', '+']).next()?;
        let mut parts = core.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        if parts.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
        })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// A version requirement: `^1.2`, `~1.2.3`, `=1.2.3`, `>=1.2`, or a bare version meaning `^`.
#[derive(Clone, Debug, PartialEq)]
pub struct VersionReq {
    op: ReqOp,
    v: Version,
    /// How many components were written, which `^` and `~` care about.
    written: u8,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum ReqOp {
    Caret,
    Tilde,
    Exact,
    AtLeast,
}

impl VersionReq {
    pub fn parse(s: &str) -> Option<VersionReq> {
        let s = s.trim();
        let (op, rest) = if let Some(r) = s.strip_prefix(">=") {
            (ReqOp::AtLeast, r)
        } else if let Some(r) = s.strip_prefix('^') {
            (ReqOp::Caret, r)
        } else if let Some(r) = s.strip_prefix('~') {
            (ReqOp::Tilde, r)
        } else if let Some(r) = s.strip_prefix('=') {
            (ReqOp::Exact, r)
        } else {
            (ReqOp::Caret, s)
        };
        let rest = rest.trim();
        let written = rest.split('.').count().min(3) as u8;
        Some(VersionReq {
            op,
            v: Version::parse(rest)?,
            written,
        })
    }

    pub fn matches(&self, v: &Version) -> bool {
        match self.op {
            ReqOp::Exact => *v == self.v,
            ReqOp::AtLeast => *v >= self.v,
            ReqOp::Tilde => {
                *v >= self.v
                    && v.major == self.v.major
                    && (self.written < 2 || v.minor == self.v.minor)
            }
            ReqOp::Caret => {
                if *v < self.v {
                    return false;
                }
                if self.v.major > 0 {
                    v.major == self.v.major
                } else if self.v.minor > 0 || self.written < 3 {
                    v.major == 0 && v.minor == self.v.minor
                } else {
                    v.major == 0 && v.minor == 0 && v.patch == self.v.patch
                }
            }
        }
    }
}

/// Nanoseconds since the Unix epoch on the wall clock.
pub fn wall_now_ns() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}

/// Format a wall-clock instant (ns since epoch) as `YYYYMMDD-HHMMSS` in UTC.
pub fn format_utc_compact(ns: i64) -> String {
    let secs = ns.div_euclid(1_000_000_000);
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{:04}{:02}{:02}-{:02}{:02}{:02}",
        y,
        m,
        d,
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// A short random-looking suffix derived from the clock and the pid; not cryptographic.
pub fn random_suffix() -> String {
    let mut x = wall_now_ns() as u64 ^ ((std::process::id() as u64) << 32);
    x ^= x >> 12;
    x ^= x << 25;
    x ^= x >> 27;
    x = x.wrapping_mul(0x2545F4914F6CDD1D);
    format!("{:04x}", (x >> 32) & 0xffff)
}

/// Walk up from `start` to find the directory containing `workspace.psl`.
pub fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    let mut dir = Some(start.to_path_buf());
    while let Some(d) = dir {
        if d.join("workspace.psl").is_file() {
            return Some(d);
        }
        dir = d.parent().map(Path::to_path_buf);
    }
    None
}

/// Make `path` relative to `root` for display, falling back to the full path.
pub fn display_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .display()
        .to_string()
}

// ----- SHA-256 (FIPS 180-4), used for manifest hashes -----

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks(64) {
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([
                chunk[4 * i],
                chunk[4 * i + 1],
                chunk[4 * i + 2],
                chunk[4 * i + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *slot = slot.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    sha256(data).iter().map(|b| format!("{:02x}", b)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let long = "a".repeat(1000);
        assert_eq!(
            sha256_hex(long.as_bytes()),
            "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
        );
    }

    #[test]
    fn versions() {
        let v = |s| Version::parse(s).unwrap();
        let r = |s| VersionReq::parse(s).unwrap();
        assert!(r("^0.1").matches(&v("0.1.3")));
        assert!(!r("^0.1").matches(&v("0.2.0")));
        assert!(r("^1.2").matches(&v("1.9.0")));
        assert!(!r("^1.2").matches(&v("2.0.0")));
        assert!(r("~1.2.3").matches(&v("1.2.9")));
        assert!(!r("~1.2.3").matches(&v("1.3.0")));
        assert!(r("=1.2.3").matches(&v("1.2.3")));
        assert!(r(">=1.2").matches(&v("3.0.0")));
        assert!(v("1.10.0") > v("1.9.9"));
    }

    #[test]
    fn dates() {
        assert_eq!(format_utc_compact(0), "19700101-000000");
        assert_eq!(
            format_utc_compact(1_793_196_612_000_000_000),
            "20261028-141012"
        );
        assert_eq!(
            format_utc_compact(951_782_400_000_000_000),
            "20000229-000000"
        );
        assert_eq!(
            format_utc_compact(4_102_444_800_000_000_000),
            "21000101-000000"
        );
    }
}
