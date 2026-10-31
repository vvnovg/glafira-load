use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{AppError, AppResult, ErrorCode};

/// Уникальное имя временного файла в том же каталоге: `<name>.<pid>.<n>.tmp`.
/// Параллельные писатели в один путь не делят временный файл.
fn tmp_path(path: &Path) -> PathBuf {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".{}.{n}.tmp", std::process::id()));
    path.with_file_name(name)
}

/// Новый файл с правами только для владельца (0600 на unix): в данных приложения бывают
/// подставленные значения `{{env}}` и заголовков. rename сохраняет права временного файла.
/// Только новый (`create_new`, `O_EXCL`): файл или symlink, подложенный заранее под этим именем,
/// не открывается, и запись не уходит по чужой ссылке.
fn create_private(path: &Path) -> std::io::Result<File> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut opts, 0o600);
    opts.open(path)
}

/// Сколько имён временного файла пробуется, пока не найдётся свободное.
const TMP_ATTEMPTS: usize = 100;

/// Временный файл: первое свободное имя из `next` (`tmp_path`). Занятое (чужой файл или symlink)
/// пропускается — следующее имя с новым номером.
fn create_tmp(mut next: impl FnMut() -> PathBuf) -> std::io::Result<(PathBuf, File)> {
    for _ in 0..TMP_ATTEMPTS {
        let tmp = next();
        match create_private(&tmp) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            r => return r.map(|f| (tmp, f)),
        }
    }
    Err(std::io::ErrorKind::AlreadyExists.into())
}

/// Атомарная запись: временный файл → fsync → rename → fsync каталога.
/// При сбое на любом шаге исходный файл остаётся нетронутым.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> AppResult<()> {
    let mut f = AtomicFile::create(path)?;
    f.write_all(bytes)?;
    f.commit()
}

fn write_failed(path: &Path, e: std::io::Error) -> AppError {
    app_err!(
        ErrorCode::Internal,
        "errors.storage.writeFailed",
        "Failed to write {{path}}: {{error}}",
        path = path.display(),
        error = e
    )
}

/// Файл, который появляется по пути только целиком, — как `atomic_write`, но данные пишутся
/// частями: во временный файл рядом (0600), `commit` — fsync, rename и fsync каталога. Без
/// `commit` (ошибка, отмена) временный файл удаляется.
pub struct AtomicFile {
    path: PathBuf,
    tmp: PathBuf,
    file: Option<File>,
}

impl AtomicFile {
    pub fn create(path: &Path) -> AppResult<Self> {
        let (tmp, file) = fs::create_dir_all(parent(path))
            .and_then(|_| create_tmp(|| tmp_path(path)))
            .map_err(|e| write_failed(path, e))?;
        Ok(Self {
            path: path.to_path_buf(),
            tmp,
            file: Some(file),
        })
    }

    pub fn write_all(&mut self, bytes: &[u8]) -> AppResult<()> {
        match &mut self.file {
            Some(f) => f.write_all(bytes).map_err(|e| write_failed(&self.path, e)),
            None => Ok(()),
        }
    }

    pub fn commit(mut self) -> AppResult<()> {
        let result = (|| -> std::io::Result<()> {
            if let Some(f) = self.file.take() {
                f.sync_all()?;
            }
            fs::rename(&self.tmp, &self.path)?;
            sync_dir(parent(&self.path));
            Ok(())
        })();
        result.map_err(|e| write_failed(&self.path, e))
    }
}

impl Drop for AtomicFile {
    /// Временного файла после rename уже нет, и удалять нечего.
    fn drop(&mut self) {
        self.file.take();
        let _ = fs::remove_file(&self.tmp);
    }
}

fn parent(path: &Path) -> &Path {
    path.parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."))
}

/// fsync каталога фиксирует rename на диске (POSIX). На Windows каталог так не открыть — пропускаем.
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// Файл записан более новой версией приложения (`schemaVersion` выше поддерживаемой).
pub fn newer_schema_error(path: &Path, version: impl std::fmt::Display) -> AppError {
    app_err!(
        ErrorCode::Internal,
        "errors.storage.newerSchema",
        "{{file}} was created by a newer version of the app (schemaVersion {{version}})",
        file = path.display(),
        version = version
    )
}

pub fn read_json<T: DeserializeOwned>(path: &Path) -> AppResult<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

/// Читает JSON; при повреждённом файле (не парсится) переименовывает его в
/// `<name>.corrupt-<unix-ms>` (сохраняя для пользователя), пишет предупреждение и возвращает None.
/// Файл с `schemaVersion` выше `max_schema` — не повреждение: возвращается ошибка (fail-closed).
pub fn read_json_or_quarantine<T: DeserializeOwned>(
    path: &Path,
    max_schema: u32,
) -> AppResult<Option<T>> {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let err = match serde_json::from_slice::<T>(&bytes) {
        Ok(v) => return Ok(Some(v)),
        Err(e) => e,
    };
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
        && v.get("schemaVersion")
            .and_then(|x| x.as_u64())
            .is_some_and(|x| x > u64::from(max_schema))
    {
        return Err(newer_schema_error(path, &v["schemaVersion"]));
    }
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(format!(".corrupt-{ms}"));
    let backup = path.with_file_name(name);
    fs::rename(path, &backup).map_err(|e| {
        app_err!(
            ErrorCode::Internal,
            "errors.storage.quarantineFailed",
            "File {{path}} is corrupted ({{parseError}}) and a backup copy could not be saved: {{renameError}}",
            path = path.display(),
            parseError = err,
            renameError = e
        )
    })?;
    tracing::warn!(
        "file {} is corrupted ({err}); saved as {}, starting empty",
        path.display(),
        backup.display()
    );
    Ok(None)
}

pub fn write_json<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    atomic_write(path, &serde_json::to_vec_pretty(value)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;

    fn leftover_tmps(dir: &Path) -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "tmp"))
            .collect()
    }

    #[test]
    fn writes_replaces_and_leaves_no_tmp() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.json");
        atomic_write(&p, b"1").unwrap();
        atomic_write(&p, b"2").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"2");
        assert!(leftover_tmps(dir.path()).is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn written_files_are_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.json");
        std::fs::write(&p, b"old").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        atomic_write(&p, b"new").unwrap();
        let mode = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "заменённый файл тоже 0600");
    }

    /// Под именем временного файла заранее лежит symlink на чужой файл: он не открывается и не
    /// обрезается, запись идёт под следующим именем, а подложенное остаётся как было.
    #[cfg(unix)]
    #[test]
    fn a_planted_symlink_is_not_followed() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        std::fs::write(&victim, b"keep").unwrap();
        let (planted, free) = (
            dir.path().join("a.json.1.tmp"),
            dir.path().join("a.json.2.tmp"),
        );
        std::os::unix::fs::symlink(&victim, &planted).unwrap();
        let mut names = [planted.clone(), free.clone()].into_iter();
        let (tmp, mut f) = create_tmp(|| names.next().unwrap()).unwrap();
        assert_eq!(tmp, free);
        f.write_all(b"secret").unwrap();
        drop(f);
        assert_eq!(
            std::fs::read(&victim).unwrap(),
            b"keep",
            "чужой файл не тронут"
        );
        assert!(std::fs::symlink_metadata(&planted).unwrap().is_symlink());
        assert_eq!(std::fs::read(&free).unwrap(), b"secret");
        // Все имена заняты — ошибка, а не запись по ссылке.
        let e = create_tmp(|| planted.clone()).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&victim).unwrap(), b"keep");
    }

    #[test]
    fn creates_missing_directories() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("history").join("c1.json");
        atomic_write(&p, b"[]").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"[]");
    }

    #[cfg(unix)]
    #[test]
    fn failure_keeps_original_file() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let sub = dir.path().join("d");
        let p = sub.join("a.json");
        atomic_write(&p, b"old").unwrap();
        // Каталог только для чтения: временный файл не создать.
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o555)).unwrap();
        let err = atomic_write(&p, b"new");
        std::fs::set_permissions(&sub, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert_eq!(err.unwrap_err().code, ErrorCode::Internal);
        assert_eq!(std::fs::read(&p).unwrap(), b"old");
        assert!(leftover_tmps(&sub).is_empty());
    }

    #[test]
    fn failed_rename_removes_tmp() {
        let dir = tempfile::tempdir().unwrap();
        // На месте целевого файла — непустой каталог: rename не пройдёт.
        let p = dir.path().join("a.json");
        std::fs::create_dir(&p).unwrap();
        std::fs::write(p.join("x"), b"x").unwrap();
        let err = atomic_write(&p, b"new").unwrap_err();
        assert_eq!(err.code, ErrorCode::Internal);
        assert!(leftover_tmps(dir.path()).is_empty());
    }

    #[test]
    fn concurrent_writers_do_not_clobber_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("h.json");
        let a = vec!["a".repeat(64); 256];
        let b = vec!["b".repeat(64); 256];
        std::thread::scope(|s| {
            for v in [&a, &b] {
                let p = &p;
                s.spawn(move || {
                    for _ in 0..100 {
                        write_json(p, v).unwrap();
                    }
                });
            }
        });
        let got: Vec<String> = read_json(&p).unwrap().unwrap();
        assert!(got == a || got == b);
        assert!(leftover_tmps(dir.path()).is_empty());
    }

    #[test]
    fn json_roundtrip_and_missing_file_is_none() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("v.json");
        assert_eq!(read_json::<Vec<i32>>(&p).unwrap(), None);
        write_json(&p, &vec![1, 2]).unwrap();
        assert_eq!(read_json::<Vec<i32>>(&p).unwrap(), Some(vec![1, 2]));
    }
}
