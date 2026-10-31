//! Версия клона сервиса через системный `git` с таймаутом (спека L1 §4).

use std::ffi::OsStr;
use std::path::Path;
use std::time::{Duration, Instant};

use serde::Serialize;

pub const TIMEOUT: Duration = Duration::from_secs(5);
pub const UNVERSIONED: &str = "unversioned";

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GitVersion {
    pub version: String,
    pub commit: String,
    pub available: bool,
    /// Почему git недоступен: нет бинаря, не репозиторий, таймаут. Текст команды, не переводится.
    pub error: Option<String>,
}

impl GitVersion {
    pub fn unversioned(error: impl Into<String>) -> Self {
        Self {
            version: UNVERSIONED.into(),
            commit: String::new(),
            available: false,
            error: Some(error.into()),
        }
    }
}

async fn run(git: &OsStr, repo: &Path, args: &[&str], timeout: Duration) -> Result<String, String> {
    let mut cmd = tokio::process::Command::new(git);
    cmd.arg("-C")
        .arg(repo)
        .args(args)
        .stdin(std::process::Stdio::null())
        // `--dirty` не должен брать `index.lock`: пока открыт проект, пользователь работает с git.
        .env("GIT_OPTIONAL_LOCKS", "0")
        // Таймаут роняет future — зависший git не остаётся висеть.
        .kill_on_drop(true);
    // Без мелькающего окна консоли при запуске из GUI на Windows (CREATE_NO_WINDOW).
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    match tokio::time::timeout(timeout, cmd.output()).await {
        Err(_) => Err(format!("git timed out after {}s", timeout.as_secs_f32())),
        Ok(Err(e)) => Err(format!("git not started: {e}")),
        Ok(Ok(out)) if out.status.success() => {
            Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
        }
        Ok(Ok(out)) => Err(String::from_utf8_lossy(&out.stderr).trim().to_string()),
    }
}

/// `timeout` — общий бюджет обеих команд: один дедлайн, а не отдельный таймаут на каждую.
async fn describe_timeout(git: &OsStr, repo: &Path, timeout: Duration) -> GitVersion {
    let deadline = Instant::now() + timeout;
    match run(
        git,
        repo,
        &["describe", "--tags", "--always", "--dirty"],
        timeout,
    )
    .await
    {
        Ok(version) => {
            let left = deadline.saturating_duration_since(Instant::now());
            let commit = run(git, repo, &["rev-parse", "HEAD"], left)
                .await
                .unwrap_or_default();
            GitVersion {
                version,
                commit,
                available: true,
                error: None,
            }
        }
        Err(e) => GitVersion::unversioned(e),
    }
}

/// `git describe --tags --always --dirty` и `git rev-parse HEAD` заданным бинарём; любая ошибка —
/// `unversioned` с причиной (это предупреждение, а не ошибка разбора или открытия).
pub async fn describe_with(git: impl AsRef<OsStr>, repo: &Path) -> GitVersion {
    describe_timeout(git.as_ref(), repo, TIMEOUT).await
}

pub async fn describe(repo: &Path) -> GitVersion {
    describe_with("git", repo).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn real_repo_tag_and_commit() {
        let dir = tempfile::tempdir().unwrap();
        let sh = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .arg("-C")
                .arg(dir.path())
                .args(args)
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{}",
                String::from_utf8_lossy(&out.stderr)
            );
        };
        sh(&["init", "-q"]);
        sh(&[
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "init",
        ]);
        sh(&["-c", "tag.gpgSign=false", "tag", "v1.2.3"]);
        let v = describe(dir.path()).await;
        assert_eq!(
            (v.version.as_str(), v.available, v.error),
            ("v1.2.3", true, None)
        );
        assert_eq!(v.commit.len(), 40);
        std::fs::write(dir.path().join("x"), "dirty").unwrap();
        sh(&["add", "x"]);
        assert_eq!(describe(dir.path()).await.version, "v1.2.3-dirty");
    }

    #[tokio::test]
    async fn missing_binary_and_plain_folder_are_unversioned() {
        let dir = tempfile::tempdir().unwrap();
        let v = describe_with("glafira-no-such-git-binary", dir.path()).await;
        assert_eq!((v.version.as_str(), v.available), (UNVERSIONED, false));
        assert!(v.error.unwrap().contains("not started"));
        let v = describe(dir.path()).await;
        assert!(!v.available, "обычная папка — не репозиторий");
        assert_eq!(v.version, UNVERSIONED);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn hanging_git_times_out() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("slow-git");
        std::fs::write(&fake, "#!/bin/sh\nsleep 30\n").unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let started = std::time::Instant::now();
        let v = describe_timeout(fake.as_os_str(), dir.path(), Duration::from_millis(200)).await;
        assert!(started.elapsed() < Duration::from_secs(5), "не ждали 30 с");
        assert!(!v.available);
        assert!(v.error.unwrap().contains("timed out"));
    }
}
