//! `GET /v1/runs/{id}/files`: tar папки прогона без сжатия (спека §4.3).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Path as AxPath, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};

use super::runs::AgentState;
use super::{api_error, not_found};

/// Обычные файлы папки (без вложенных) по алфавиту, имена без путей.
pub fn tar_dir(dir: &Path) -> std::io::Result<Vec<u8>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    let mut builder = tar::Builder::new(Vec::new());
    for path in files {
        let Some(name) = path.file_name() else {
            continue;
        };
        builder.append_path_with_name(&path, name)?;
    }
    builder.into_inner()
}

/// Файлы отдаются только завершённому прогону: пока он идёт или собирает файлы `LocalAgent`,
/// tar был бы неполным — 409 `AGENT_BUSY` (контроллер Task 11 берёт файлы после `done`).
pub async fn files(
    State(state): State<Arc<AgentState>>,
    AxPath(run_id): AxPath<String>,
) -> Response {
    // Сначала текущий прогон, затем история: `finish` переносит прогон под замком `current`,
    // так что ушедший из текущих уже виден в истории.
    if state.busy().as_deref() == Some(run_id.as_str()) {
        return api_error(
            StatusCode::CONFLICT,
            "AGENT_BUSY",
            "run is still in progress",
        );
    }
    let Some(dir) = state.history_dir(&run_id).filter(|d| d.is_dir()) else {
        return not_found(&run_id);
    };
    match tokio::task::spawn_blocking(move || tar_dir(&dir)).await {
        Ok(Ok(bytes)) => ([(header::CONTENT_TYPE, "application/x-tar")], bytes).into_response(),
        Ok(Err(e)) => api_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "IO",
            format!("cannot pack run files: {e}"),
        ),
        Err(e) => api_error(StatusCode::INTERNAL_SERVER_ERROR, "IO", e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::tar_dir;

    #[test]
    fn packs_plain_files_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("summary.yaml"), "status: completed\n").unwrap();
        std::fs::write(dir.path().join("agent.log"), "hi\n").unwrap();
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let bytes = tar_dir(dir.path()).unwrap();
        let names: Vec<String> = tar::Archive::new(&bytes[..])
            .entries()
            .unwrap()
            .map(|e| e.unwrap().path().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["agent.log", "summary.yaml"]);
    }
}
