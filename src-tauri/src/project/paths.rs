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

/// Канонический путь существующего объекта. На Windows без префикса `\\?\` (`dunce`), чтобы
/// пути оставались привычными и сравнимыми по тексту.
pub fn canonical(p: &Path) -> AppResult<PathBuf> {
    dunce::canonicalize(p).map_err(|e| AppError::from(e).with_details(p.display().to_string()))
}

/// Канонический вид пути, которого может ещё не быть: ближайший существующий предок
/// канонизируется, остаток добавляется без обращения к диску, `..` в остатке снимается лексически.
/// Висячая ссылка (`exists()` её не видит, цель не существует) попадает в остаток как обычное имя:
/// это безопасно, потому что запись делает rename временного файла поверх самой ссылки и не
/// идёт по ней, а создание каталога под ссылкой завершается ошибкой.
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

/// Путь относительно канонического `base` с разделителем `/` (для манифеста и реестра). Вне `base`
/// (нет префикса или в остатке есть `..`) — абсолютный как есть. И `base`, и `abs` должны быть
/// каноническими (`ProjectDir::root`, результат `inside`), иначе сравнение по тексту даст абсолютный путь.
pub fn relative(base: &Path, abs: &Path) -> String {
    match abs.strip_prefix(base) {
        Ok(rel) if rel.components().all(|c| matches!(c, Component::Normal(_))) => rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
        _ => abs.to_string_lossy().into_owned(),
    }
}

/// Путь проекта из команд должен быть абсолютным: относительный зависел бы от cwd приложения.
pub fn require_absolute(path: &str) -> AppResult<()> {
    if Path::new(path).is_absolute() {
        Ok(())
    } else {
        Err(app_err!(
            ErrorCode::InvalidInput,
            "errors.project.pathNotAbsolute",
            "Project path must be absolute: {{path}}",
            path = path
        ))
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
        // Абсолютный путь своей платформы: `/abs` на Windows не абсолютный (нет диска).
        let abs = std::env::temp_dir();
        assert_eq!(resolve(base, abs.to_str().unwrap()), abs);
    }

    #[test]
    fn missing_parents_are_checked_lexically() {
        let dir = tempfile::tempdir().unwrap();
        // Корень канонизирует сам ProjectDir: тест не канонизирует путь вручную.
        let path = dir.path().join("p");
        crate::project::manifest::create(path.to_str().unwrap(), "x").unwrap();
        let d = crate::project::manifest::ProjectDir::open(path.to_str().unwrap()).unwrap();
        let root = d.root();
        let ok = inside(root, &d.file("new/deeper/f.txt")).unwrap();
        assert_eq!(
            ok,
            root.join("new/deeper/f.txt"),
            "несуществующие папки внутри — ок"
        );
        assert_eq!(
            relative(root, &ok),
            "new/deeper/f.txt",
            "внутри — относительный"
        );
        let e = inside(root, &d.file("missing/../../outside.txt")).unwrap_err();
        assert_eq!(e.code, ErrorCode::PathOutsideProject);
        assert_eq!(
            relative(root, Path::new("/elsewhere/x")),
            "/elsewhere/x",
            "вне корня — абсолютный"
        );
        let clone = root.join("../clone");
        assert_eq!(
            relative(root, &clone),
            clone.to_string_lossy(),
            "путь с `..` не превращается в относительный"
        );
    }

    #[test]
    fn project_path_must_be_absolute() {
        assert!(require_absolute(std::env::temp_dir().to_str().unwrap()).is_ok());
        let e = require_absolute("rel/dir").unwrap_err();
        assert_eq!(e.code, ErrorCode::InvalidInput);
        assert_eq!(e.key, "errors.project.pathNotAbsolute");
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

    /// Висячая ссылка внутри проекта как цель записи: rename заменяет саму ссылку, до её цели
    /// (в том числе снаружи проекта) запись не доходит; запись «под» такой ссылкой невозможна.
    #[cfg(unix)]
    #[test]
    fn dangling_symlink_cannot_redirect_a_write() {
        use crate::project::manifest::{ProjectDir, create};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p");
        create(path.to_str().unwrap(), "x").unwrap();
        let d = ProjectDir::open(path.to_str().unwrap()).unwrap();
        let outside = dir.path().join("outside");
        std::os::unix::fs::symlink(&outside, path.join("dangling")).unwrap();
        d.write_text("dangling", "data").unwrap();
        assert!(!outside.exists(), "цель ссылки не создана");
        let meta = std::fs::symlink_metadata(path.join("dangling")).unwrap();
        assert!(meta.is_file(), "ссылка заменена обычным файлом");
        assert_eq!(
            std::fs::read_to_string(path.join("dangling")).unwrap(),
            "data"
        );

        let outside_dir = dir.path().join("outside-dir");
        std::os::unix::fs::symlink(&outside_dir, path.join("dir-link")).unwrap();
        assert!(d.write_text("dir-link/f.txt", "data").is_err());
        assert!(!outside_dir.exists(), "каталог по ссылке не создан");
    }
}
