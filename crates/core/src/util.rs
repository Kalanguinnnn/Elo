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

/// Hide credentials embedded in URLs (`https://user:token@host/x` →
/// `https://***@host/x`) anywhere in `text`: Git error messages, logs and
/// status may carry remote URLs configured before Elo rejected them.
pub fn redact_url_credentials(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c == '/' || c == '\'' || c == '"' || c == '<' || c == '>' || c.is_whitespace())
            .unwrap_or(tail.len());
        match tail[..end].rfind('@') {
            Some(at) => {
                out.push_str("***");
                out.push_str(&tail[at..end]);
            }
            None => out.push_str(&tail[..end]),
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Whether the process `pid` is still running. Used to wait until the Agent
/// has really exited before an update replaces its executable.
#[cfg(windows)]
pub fn process_alive(pid: u32) -> bool {
    use windows_sys::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    // SAFETY: plain OpenProcess/GetExitCodeProcess; a null handle is handled
    // and the handle is closed before returning.
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return false;
        }
        let mut code = 0u32;
        let ok = GetExitCodeProcess(h, &mut code) != 0;
        CloseHandle(h);
        ok && code == STILL_ACTIVE as u32
    }
}

#[cfg(not(windows))]
pub fn process_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_userinfo_in_urls() {
        assert_eq!(redact_url_credentials("https://user:tok@github.com/a/b.git"), "https://***@github.com/a/b.git");
        assert_eq!(redact_url_credentials("https://ghp_x@github.com/a"), "https://***@github.com/a");
        assert_eq!(
            redact_url_credentials("fatal: unable to access 'https://u:p@h.io/r/': 403"),
            "fatal: unable to access 'https://***@h.io/r/': 403"
        );
        assert_eq!(redact_url_credentials("https://github.com/a/b x@y"), "https://github.com/a/b x@y");
        assert_eq!(redact_url_credentials("git@github.com:a/b.git"), "git@github.com:a/b.git");
        assert_eq!(redact_url_credentials("a https://x:y@h b http://k@h2/p"), "a https://***@h b http://***@h2/p");
        assert_eq!(redact_url_credentials("sem url"), "sem url");
    }

    #[test]
    fn process_alive_tracks_a_child_until_it_exits() {
        assert!(process_alive(std::process::id()));
        let mut child = std::process::Command::new("git").arg("--version").stdout(std::process::Stdio::null()).spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        drop(child);
        assert!(!process_alive(pid));
    }
}
