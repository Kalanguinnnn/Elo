//! Path-safety rules for everything Continuity writes into a working tree or a
//! restore directory. Tree paths come from remote data and are untrusted.

use crate::error::{Error, Result};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
    "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Validate a repository-relative path (always `/`-separated, as git stores it).
pub fn validate_rel_path(p: &str) -> Result<()> {
    let fail = |why: &str| Err(Error::UnsafePath(format!("{p:?}: {why}")));
    if p.is_empty() {
        return fail("vazio");
    }
    if p.contains('\0') || p.contains('\\') {
        return fail("caractere proibido");
    }
    if p.starts_with('/') || p.len() >= 2 && p.as_bytes()[1] == b':' {
        return fail("absoluto");
    }
    for comp in p.split('/') {
        if comp.is_empty() || comp == "." || comp == ".." {
            return fail("componente inválido");
        }
        let lower = comp.to_ascii_lowercase();
        // `.git` in any case, plus the NTFS short name and trailing-dot/space
        // aliases that Windows resolves to the same directory.
        let trimmed = lower.trim_end_matches(['.', ' ']);
        if trimmed == ".git" || trimmed == "git~1" {
            return fail("componente .git");
        }
        if comp.contains(':') {
            return fail("dois-pontos (ADS/unidade)");
        }
        // Names that Windows aliases or refuses. Harmless on Linux, where they
        // stay writable.
        if !cfg!(windows) {
            continue;
        }
        if comp.ends_with('.') || comp.ends_with(' ') {
            return fail("termina com ponto ou espaço");
        }
        let stem = comp.split('.').next().unwrap_or("").to_ascii_uppercase();
        if WINDOWS_RESERVED.contains(&stem.as_str()) {
            return fail("nome reservado do Windows");
        }
        if comp.chars().any(|c| matches!(c, '<' | '>' | '"' | '|' | '?' | '*') || (c as u32) < 32) {
            return fail("caractere inválido no Windows");
        }
    }
    Ok(())
}

/// Join a validated relative path to `root`.
pub fn join_rel(root: &Path, rel: &str) -> Result<PathBuf> {
    validate_rel_path(rel)?;
    let mut out = root.to_path_buf();
    for comp in rel.split('/') {
        out.push(comp);
    }
    // Defence in depth: the joined path must only contain normal components
    // below root.
    let tail = out.strip_prefix(root).map_err(|_| Error::UnsafePath(rel.into()))?;
    if tail.components().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(Error::UnsafePath(rel.into()));
    }
    Ok(out)
}

/// Ensure no existing ancestor of `target` (strictly below `root`) is a symlink
/// or a non-directory. Prevents writing through a planted link.
pub fn check_no_symlink_ancestors(root: &Path, target: &Path) -> Result<()> {
    let rel = target.strip_prefix(root).map_err(|_| Error::UnsafePath(target.display().to_string()))?;
    let mut cur = root.to_path_buf();
    let comps: Vec<_> = rel.components().collect();
    for comp in comps.iter().take(comps.len().saturating_sub(1)) {
        cur.push(comp);
        match fs::symlink_metadata(&cur) {
            Ok(md) => {
                if md.file_type().is_symlink() || is_reparse_point(&md) {
                    return Err(Error::UnsafePath(format!("{} é um link simbólico", cur.display())));
                }
                if !md.is_dir() {
                    return Err(Error::UnsafePath(format!("{} não é diretório", cur.display())));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_point(md: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    md.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}
#[cfg(not(windows))]
fn is_reparse_point(_md: &fs::Metadata) -> bool {
    false
}

/// Whether a symlink target stays inside the tree when resolved lexically from
/// the link's own directory.
pub fn symlink_target_is_contained(link_rel: &str, target: &str) -> bool {
    if target.is_empty() || target.starts_with('/') || target.contains('\\') || target.contains(':') {
        return false;
    }
    let mut depth: i64 = link_rel.split('/').count() as i64 - 1;
    for comp in target.split('/') {
        match comp {
            "" | "." => {}
            ".." => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            c if c.eq_ignore_ascii_case(".git") => return false,
            _ => depth += 1,
        }
    }
    true
}

/// Atomically write `data` to `target` (temp file in the same directory, then
/// rename). Replaces a symlink at `target` instead of following it.
pub fn atomic_write(root: &Path, target: &Path, data: &[u8], executable: bool) -> Result<()> {
    check_no_symlink_ancestors(root, target)?;
    let dir = target.parent().ok_or_else(|| Error::UnsafePath(target.display().to_string()))?;
    fs::create_dir_all(dir)?;
    check_no_symlink_ancestors(root, target)?;
    if let Ok(md) = fs::symlink_metadata(target) {
        if md.is_dir() {
            return Err(Error::Conflict(format!("{} é um diretório", target.display())));
        }
        if md.file_type().is_symlink() {
            fs::remove_file(target)?;
        }
    }
    let tmp = dir.join(format!(".continuity-tmp-{}", crate::util::random_hex(6)));
    {
        let mut f = fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(data)?;
        f.sync_all()?;
    }
    set_executable(&tmp, executable)?;
    if let Err(e) = rename_replace(&tmp, target) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(())
}

fn rename_replace(from: &Path, to: &Path) -> Result<()> {
    // On Windows a file opened without FILE_SHARE_DELETE by an editor makes the
    // rename fail transiently; retry briefly.
    let mut last = None;
    for _ in 0..5 {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                std::thread::sleep(std::time::Duration::from_millis(60));
            }
        }
    }
    Err(last.map(Error::Io).unwrap_or_else(|| Error::Other("rename".into())))
}

#[cfg(unix)]
fn set_executable(p: &Path, exec: bool) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mode = if exec { 0o755 } else { 0o644 };
    fs::set_permissions(p, fs::Permissions::from_mode(mode))?;
    Ok(())
}
#[cfg(not(unix))]
fn set_executable(_p: &Path, _exec: bool) -> Result<()> {
    Ok(())
}

/// Create a symlink when the platform allows it and the target is contained;
/// otherwise write a plain file holding the target (what Git for Windows does
/// with `core.symlinks=false`).
pub fn write_symlink(root: &Path, target_path: &Path, link_rel: &str, link_target: &str) -> Result<()> {
    if !symlink_target_is_contained(link_rel, link_target) {
        return Err(Error::UnsafePath(format!("symlink {link_rel} -> {link_target} sai do projeto")));
    }
    #[cfg(unix)]
    {
        check_no_symlink_ancestors(root, target_path)?;
        if let Some(dir) = target_path.parent() {
            fs::create_dir_all(dir)?;
        }
        if let Ok(md) = fs::symlink_metadata(target_path) {
            if md.is_dir() {
                return Err(Error::Conflict(format!("{} é um diretório", target_path.display())));
            }
            fs::remove_file(target_path)?;
        }
        std::os::unix::fs::symlink(link_target, target_path)?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        atomic_write(root, target_path, link_target.as_bytes(), false)
    }
}

/// Remove a file and then any directories left empty, never climbing above root.
pub fn remove_file_and_empty_parents(root: &Path, target: &Path) -> Result<()> {
    check_no_symlink_ancestors(root, target)?;
    match fs::symlink_metadata(target) {
        Ok(md) if md.is_dir() => return Err(Error::Conflict(format!("{} é um diretório", target.display()))),
        Ok(_) => fs::remove_file(target)?,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut dir = target.parent();
    while let Some(d) = dir {
        if d == root || !d.starts_with(root) {
            break;
        }
        if fs::remove_dir(d).is_err() {
            break; // not empty (or busy): stop
        }
        dir = d.parent();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_traversal_and_special_names() {
        for bad in [
            "../x", "a/../../x", "/etc/passwd", "C:/x", "a\\b", ".git/config", "a/.GIT/hooks/x", ".git./x",
            "git~1/x", "a//b", "./a", "a:stream", "x/",
        ] {
            assert!(validate_rel_path(bad).is_err(), "{bad} deveria ser rejeitado");
        }
        if cfg!(windows) {
            for bad in ["con", "dir/NUL.txt", "trail. ", "q?"] {
                assert!(validate_rel_path(bad).is_err(), "{bad} deveria ser rejeitado");
            }
        }
        for ok in ["a.txt", "src/main.rs", "dir with space/file.md", ".github/workflows/ci.yml", ".gitignore", "conf/x"] {
            assert!(validate_rel_path(ok).is_ok(), "{ok} deveria ser aceito");
        }
    }

    #[test]
    fn symlink_containment() {
        assert!(symlink_target_is_contained("a/link", "../b"));
        assert!(symlink_target_is_contained("link", "dir/file"));
        assert!(!symlink_target_is_contained("link", "../outside"));
        assert!(!symlink_target_is_contained("a/link", "../../x"));
        assert!(!symlink_target_is_contained("link", "/etc/passwd"));
        assert!(!symlink_target_is_contained("link", ".git/config"));
    }

    #[test]
    fn atomic_write_and_remove() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let t = join_rel(root, "x/y/z.txt").unwrap();
        atomic_write(root, &t, b"hello", false).unwrap();
        assert_eq!(fs::read(&t).unwrap(), b"hello");
        atomic_write(root, &t, b"again", false).unwrap();
        assert_eq!(fs::read(&t).unwrap(), b"again");
        remove_file_and_empty_parents(root, &t).unwrap();
        assert!(!root.join("x").exists());
        assert!(root.exists());
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_parent() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::os::unix::fs::symlink(outside.path(), root.join("evil")).unwrap();
        let t = join_rel(root, "evil/payload.txt").unwrap();
        assert!(atomic_write(root, &t, b"x", false).is_err());
        assert!(!outside.path().join("payload.txt").exists());
    }
}
