//! Границы проекта и клонов: любой путь чтения или записи проверяется (спека L1 §3.3, §11).

use std::path::{Component, Path, PathBuf};

use crate::error::{AppError, AppResult, ErrorCode};

/// `rel` относительно `base`, абсолютный — как есть.
pub fn resolve(base: &Path, rel: &str) -> PathBuf {
    let p = Path::new(rel);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        base.join(p)
    }
}

fn canonical(p: &Path) -> AppResult<PathBuf> {
    p.canonicalize()
        .map_err(|e| AppError::from(e).with_details(p.display().to_string()))
}

/// Канонический вид пути, которого может ещё не быть: ближайший существующий предок
/// канонизируется, остаток добавляется без обращения к диску (несуществующие части не бывают
/// ссылками, поэтому `..` в остатке снимается лексически).
fn canonical_lenient(p: &Path) -> AppResult<PathBuf> {
    let parts: Vec<Component> = p.components().collect();
    let mut i = parts.len();
    let existing = loop {
        let prefix: PathBuf = parts[..i].iter().collect();
        let prefix = if prefix.as_os_str().is_empty() {
            PathBuf::from(".")
        } else {
            prefix
        };
        if prefix.exists() || i == 0 {
            break prefix;
        }
        i -= 1;
    };
    let mut out = canonical(&existing)?;
    for c in &parts[i..] {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    Ok(out)
}

/// Канонический `candidate`, если он внутри `base`. Несуществующий путь проверяется по ближайшему
/// существующему предку (так проверяются пути записи).
pub fn inside(base: &Path, candidate: &Path) -> AppResult<PathBuf> {
    let base_c = canonical(base)?;
    let target = canonical_lenient(candidate)?;
    if target.starts_with(&base_c) {
        Ok(target)
    } else {
        Err(app_err!(
            ErrorCode::PathOutsideProject,
            "errors.project.outside",
            "Path {{path}} is outside {{base}}",
            path = candidate.display(),
            base = base.display()
        ))
    }
}

/// Путь относительно `base` с разделителем `/` (для манифеста и реестра); вне `base` — абсолютный.
pub fn relative(base: &Path, abs: &Path) -> String {
    match abs.strip_prefix(base) {
        Ok(rel) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => abs.to_string_lossy().into_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inside_accepts_children_and_rejects_escapes() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::fs::create_dir_all(base.join("a/b")).unwrap();
        std::fs::write(base.join("a/b/f.txt"), "x").unwrap();
        assert!(inside(base, &base.join("a/b/f.txt")).is_ok());
        assert!(
            inside(base, &base.join("a/new.txt")).is_ok(),
            "несуществующий файл внутри — ок"
        );
        let e = inside(base, &base.join("a/../../outside.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideProject);
        assert_eq!(relative(base, &base.join("a/b/f.txt")), "a/b/f.txt");
        assert_eq!(resolve(base, "a"), base.join("a"));
        assert_eq!(resolve(base, "/abs"), PathBuf::from("/abs"));
    }

    #[test]
    fn missing_parents_are_checked_lexically() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path();
        let ok = inside(base, &base.join("new/deeper/f.txt")).unwrap();
        assert_eq!(
            ok,
            base.canonicalize().unwrap().join("new/deeper/f.txt"),
            "несуществующие папки внутри — ок"
        );
        let e = inside(base, &base.join("missing/../../outside.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideProject);
        assert_eq!(
            relative(base, Path::new("/elsewhere/x")),
            "/elsewhere/x",
            "вне base — абсолютный"
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlink_out_of_the_base_is_outside() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let base = dir.path();
        std::os::unix::fs::symlink(other.path(), base.join("link")).unwrap();
        let e = inside(base, &base.join("link/f.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideProject);
    }
}
