use chrono::{DateTime, Utc};

pub fn random_bytes<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    getrandom::fill(&mut b).expect("OS RNG unavailable");
    b
}

pub fn random_hex(bytes: usize) -> String {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).expect("OS RNG unavailable");
    hex::encode(b)
}

pub fn now() -> DateTime<Utc> {
    Utc::now()
}

pub fn now_ms() -> i64 {
    Utc::now().timestamp_millis()
}

pub fn ts_to_rfc3339(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms).unwrap_or_default().to_rfc3339()
}

/// Single-line, length-bounded text safe to embed in a commit trailer.
pub fn sanitize_line(s: &str, max: usize) -> String {
    let mut out: String = s.chars().filter(|c| !c.is_control()).collect();
    out = out.trim().to_string();
    if out.chars().count() > max {
        out = out.chars().take(max).collect();
    }
    out
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}
