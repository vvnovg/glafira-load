//! Исполнитель HTTP-шагов на `reqwest` с `rustls` (спека L2 §5.2): пул соединений,
//! без редиректов, проверки `expect`, извлечение `extract`, примеры ошибок с маскированием.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use glafira_load_model::schema::SchemaSummary;
use glafira_load_model::suite::expr::{
    Context, ExprError, parse_template, render_json, render_string,
};
use glafira_load_model::suite::{Action, HttpDefaults, HttpStep, Step};
use percent_encoding::{AsciiSet, NON_ALPHANUMERIC, utf8_percent_encode};
use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderName, HeaderValue};
use reqwest::{Client, Method, redirect};
use serde_json::Value;

use crate::context::IterationCtx;
use crate::errors::{StepCode, StepError, sample};
use crate::executor::{StepExecutor, StepOutcome};
use crate::expect::check_expect;
use crate::extract::extract_all;

/// Сегмент пути: кодируется всё, кроме unreserved RFC 3986.
const PATH_SEGMENT: &AsciiSet = &NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// Все формы записи секретов, которые могут попасть в URL примера ошибки: исходное значение,
/// сегмент пути (`PATH_SEGMENT`), `application/x-www-form-urlencoded` (значения query в reqwest)
/// и полное percent-кодирование. Пустые значения и дубликаты отбрасываются.
fn secret_variants(secrets: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for s in secrets.iter().filter(|s| !s.is_empty()) {
        let forms = [
            s.clone(),
            utf8_percent_encode(s, PATH_SEGMENT).to_string(),
            utf8_percent_encode(s, NON_ALPHANUMERIC).to_string(),
            form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>(),
        ];
        for f in forms {
            if !out.contains(&f) {
                out.push(f);
            }
        }
    }
    out
}

/// Метод и шаблон пути из id операции `http:<service>:<METHOD> <path>`.
pub fn parse_operation(id: &str) -> Option<(Method, &str)> {
    let rest = id.strip_prefix("http:")?;
    let (_service, call) = rest.split_once(':')?;
    let (method, path) = call.split_once(' ')?;
    let method = Method::from_bytes(method.as_bytes()).ok()?;
    if path.is_empty() {
        return None;
    }
    Some((method, path))
}

fn expr_error(e: ExprError) -> StepError {
    StepError::new(
        StepCode::ExtractMissing,
        format!("expression: {}", e.message),
    )
}

fn build_error(message: impl Into<String>) -> StepError {
    StepError::new(
        StepCode::Connect,
        format!("cannot build request: {}", message.into()),
    )
}

fn render_str(text: &str, ctx: &dyn Context) -> Result<String, StepError> {
    render_string(&parse_template(text).map_err(expr_error)?, ctx).map_err(expr_error)
}

/// Подставляет `{name}` шаблона пути значениями `params` (после рендера выражений),
/// кодируя их как сегмент пути.
pub fn build_path(
    template: &str,
    params: &BTreeMap<String, String>,
    ctx: &dyn Context,
) -> Result<String, StepError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| build_error(format!("unclosed `{{` in path `{template}`")))?;
        let name = &after[..end];
        let raw = params
            .get(name)
            .ok_or_else(|| build_error(format!("path parameter `{name}` is not set")))?;
        let value = render_str(raw, ctx)?;
        out.extend(utf8_percent_encode(&value, PATH_SEGMENT));
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
}

fn is_json(content_type: Option<&HeaderValue>) -> bool {
    content_type
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.to_ascii_lowercase().contains("json"))
}

fn header_lines(headers: &HeaderMap) -> String {
    headers
        .iter()
        .map(|(k, v)| format!("{k}: {}", v.to_str().unwrap_or("<binary>")))
        .collect::<Vec<_>>()
        .join("\n")
}

fn map_send_error(e: reqwest::Error, timeout_ms: u64) -> StepError {
    if e.is_timeout() {
        StepError::new(StepCode::Timeout, format!("no response in {timeout_ms} ms"))
    } else if e.is_connect() {
        StepError::new(StepCode::Connect, format!("connection failed: {e}"))
    } else {
        StepError::new(StepCode::Connect, e.to_string())
    }
}

pub struct HttpExecutor {
    client: Client,
    base_url: String,
    defaults: HttpDefaults,
    /// Секреты во всех формах кодирования (см. `secret_variants`) для маскирования примеров.
    secrets: Vec<String>,
    schemas: BTreeMap<String, SchemaSummary>,
}

impl HttpExecutor {
    /// `base_url` — адрес стенда (`env.http`), `secrets` — значения для маскирования.
    pub fn new(
        base_url: &str,
        defaults: &HttpDefaults,
        secrets: &[String],
    ) -> Result<Self, String> {
        let trimmed = base_url.trim_end_matches('/');
        let parsed = reqwest::Url::parse(trimmed)
            .map_err(|e| format!("invalid base url `{base_url}`: {e}"))?;
        if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
            return Err(format!(
                "invalid base url `{base_url}`: expected http(s)://host[:port]"
            ));
        }
        let client = Client::builder()
            .redirect(redirect::Policy::none())
            .pool_max_idle_per_host(1024)
            .tcp_keepalive(Duration::from_secs(30))
            .build()
            .map_err(|e| format!("http client: {e}"))?;
        Ok(Self {
            client,
            base_url: trimmed.to_string(),
            defaults: defaults.clone(),
            secrets: secret_variants(secrets),
            schemas: BTreeMap::new(),
        })
    }

    /// Схемы ответов операций из реестра — для `expect.schema: response`.
    pub fn with_schemas(mut self, schemas: BTreeMap<String, SchemaSummary>) -> Self {
        self.schemas = schemas;
        self
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Ошибка с замаскированным сообщением (URL в тексте reqwest может содержать секрет).
    fn masked(&self, e: StepError) -> StepError {
        StepError {
            message: sample(&e.message, &self.secrets),
            ..e
        }
    }

    async fn run_http(
        &self,
        step: &Step,
        http: &HttpStep,
        ctx: &mut IterationCtx<'_>,
    ) -> Result<StepOutcome, StepError> {
        let (method, path_template) = parse_operation(&http.operation)
            .ok_or_else(|| build_error(format!("bad http operation id `{}`", http.operation)))?;
        let path = build_path(path_template, &http.path, ctx)?;
        let url = format!("{}{path}", self.base_url);
        let timeout_ms = http.timeout_ms.unwrap_or(self.defaults.timeout_ms);

        let mut query = Vec::with_capacity(http.query.len());
        for (k, v) in &http.query {
            query.push((k.clone(), render_str(v, ctx)?));
        }
        let mut headers = HeaderMap::new();
        for (k, v) in self.defaults.headers.iter().chain(http.headers.iter()) {
            let name = HeaderName::from_bytes(k.as_bytes())
                .map_err(|_| build_error(format!("bad header name `{k}`")))?;
            let value = HeaderValue::from_str(&render_str(v, ctx)?)
                .map_err(|_| build_error(format!("bad header value for `{k}`")))?;
            headers.insert(name, value);
        }
        let body: Option<Vec<u8>> = if let Some(json) = &http.body {
            let rendered = render_json(json, ctx).map_err(expr_error)?;
            if !headers.contains_key(CONTENT_TYPE) {
                headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
            }
            Some(serde_json::to_vec(&rendered).map_err(|e| build_error(e.to_string()))?)
        } else if let Some(text) = &http.body_text {
            Some(render_str(text, ctx)?.into_bytes())
        } else {
            None
        };
        let bytes_out = body.as_ref().map(|b| b.len() as u64).unwrap_or(0);

        let mut builder = self
            .client
            .request(method.clone(), &url)
            .query(&query)
            .headers(headers.clone())
            .timeout(Duration::from_millis(timeout_ms));
        if let Some(b) = &body {
            builder = builder.body(b.clone());
        }
        let request = builder
            .build()
            .map_err(|e| self.masked(build_error(e.to_string())))?;
        let full_url = request.url().to_string();
        let request_sample = || {
            let body_text = body
                .as_deref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            sample(
                &format!(
                    "{method} {full_url}\n{}\n\n{body_text}",
                    header_lines(&headers)
                ),
                &self.secrets,
            )
        };

        let started = Instant::now();
        let response = self.client.execute(request).await.map_err(|e| {
            self.masked(map_send_error(e, timeout_ms))
                .with_sample(Some(request_sample()), None)
        })?;
        let status = response.status().as_u16();
        let resp_headers = response.headers().clone();
        let bytes = response.bytes().await.map_err(|e| {
            // Тело не прочитано: в примере ответа остаются статус и заголовки.
            let head = sample(
                &format!("{status}\n{}", header_lines(&resp_headers)),
                &self.secrets,
            );
            self.masked(map_send_error(e, timeout_ms))
                .with_sample(Some(request_sample()), Some(head))
        })?;
        let latency = started.elapsed();
        let latency_us = latency.as_micros() as u64;
        let latency_ms = latency.as_secs_f64() * 1000.0;
        let bytes_in = bytes.len() as u64;
        let parsed: Option<Value> = if is_json(resp_headers.get(CONTENT_TYPE)) {
            serde_json::from_slice(&bytes).ok()
        } else {
            None
        };
        let response_sample = || {
            sample(
                &format!(
                    "{status}\n{}\n\n{}",
                    header_lines(&resp_headers),
                    String::from_utf8_lossy(&bytes)
                ),
                &self.secrets,
            )
        };
        let with_samples =
            |e: StepError| e.with_sample(Some(request_sample()), Some(response_sample()));

        if step.expect.status.is_none() && status >= 400 {
            let code = if status >= 500 {
                StepCode::Http5xx
            } else {
                StepCode::Http4xx
            };
            return Err(with_samples(StepError::new(
                code,
                format!("status {status}"),
            )));
        }
        check_expect(
            &step.expect,
            Some(status),
            parsed.as_ref(),
            latency_ms,
            self.schemas.get(&http.operation),
        )
        .map_err(with_samples)?;
        let extracted = extract_all(&step.extract, parsed.as_ref()).map_err(with_samples)?;
        for (name, value) in extracted {
            ctx.store(&name, value);
        }
        Ok(StepOutcome {
            latency_us,
            bytes_in,
            bytes_out,
            body: parsed,
            status: Some(status),
        })
    }
}

#[async_trait]
impl StepExecutor for HttpExecutor {
    async fn execute(
        &self,
        step: &Step,
        ctx: &mut IterationCtx<'_>,
    ) -> Result<StepOutcome, StepError> {
        match &step.action {
            Action::Http(http) => self.run_http(step, http, ctx).await,
            other => Err(StepError::new(
                StepCode::Connect,
                format!("http executor cannot run a {} step", other.kind().as_str()),
            )),
        }
    }

    async fn lag(&self) -> BTreeMap<String, i64> {
        BTreeMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_api::EnvAddresses;
    use crate::context::{DataFormat, RunContext, parse_table};
    use axum::Router;
    use axum::extract::{Json, Path, Query};
    use axum::http::{HeaderMap as AxumHeaders, StatusCode, header::LOCATION};
    use axum::routing::{get, post};
    use glafira_load_model::suite::{ExistsTag, Expect, Matcher, SchemaExpect, StatusExpect};
    use serde_json::json;
    use std::collections::HashMap;

    /// Заглушка стенда на свободном порту.
    async fn stub() -> String {
        let app = Router::new()
            .route(
                "/v1/orders",
                post(|Json(body): Json<Value>| async move {
                    (
                        StatusCode::CREATED,
                        Json(json!({"id": "A-1", "echo": body})),
                    )
                }),
            )
            .route(
                "/v1/orders/{id}",
                get(
                    |Path(id): Path<String>,
                     Query(q): Query<HashMap<String, String>>,
                     headers: AxumHeaders| async move {
                        let h: HashMap<String, String> = headers
                            .iter()
                            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or("").to_string()))
                            .collect();
                        Json(json!({"id": id, "query": q, "headers": h}))
                    },
                ),
            )
            .route(
                "/fail",
                get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
            )
            .route(
                "/v1/fail/{id}",
                get(|| async { (StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
            )
            .route(
                "/teapot",
                get(|| async { (StatusCode::IM_A_TEAPOT, "short and stout") }),
            )
            .route(
                "/slow",
                get(|| async {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    "late"
                }),
            )
            .route(
                "/redirect",
                get(|| async { (StatusCode::FOUND, [(LOCATION, "/v1/orders/1")], "") }),
            )
            .route("/text", get(|| async { "plain" }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://{addr}")
    }

    fn run_ctx() -> RunContext {
        RunContext {
            run_id: "run-1".into(),
            agent_id: "gen-1".into(),
            env: EnvAddresses::default(),
            secrets: BTreeMap::from([("API_TOKEN".to_string(), "s3cr3t".to_string())]),
            data: BTreeMap::from([
                (
                    "users".to_string(),
                    parse_table(DataFormat::Csv, "id\n7\n").unwrap(),
                ),
                (
                    "skus".to_string(),
                    parse_table(DataFormat::JsonLines, "{\"sku\":\"S1\"}\n").unwrap(),
                ),
            ]),
        }
    }

    fn executor(base: &str) -> HttpExecutor {
        let defaults = HttpDefaults {
            timeout_ms: 2000,
            headers: BTreeMap::from([("Accept".to_string(), "application/json".to_string())]),
        };
        HttpExecutor::new(base, &defaults, &["s3cr3t".to_string()]).unwrap()
    }

    fn http_step(operation: &str) -> HttpStep {
        HttpStep {
            operation: operation.into(),
            path: BTreeMap::new(),
            query: BTreeMap::new(),
            headers: BTreeMap::new(),
            body: None,
            body_text: None,
            timeout_ms: None,
        }
    }

    fn step(http: HttpStep, expect: Expect, extract: &[(&str, &str)]) -> Step {
        Step {
            id: Some("s".into()),
            action: Action::Http(http),
            expect,
            extract: extract
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    fn get_step(path: &str) -> Step {
        step(
            http_step(&format!("http:orders:GET {path}")),
            Expect::default(),
            &[],
        )
    }

    #[test]
    fn operation_id_gives_method_and_path() {
        let (m, p) = parse_operation("http:orders:POST /v1/orders").unwrap();
        assert_eq!((m, p), (Method::POST, "/v1/orders"));
        let (m, p) = parse_operation("http:orders:GET /v1/orders/{id}").unwrap();
        assert_eq!((m, p), (Method::GET, "/v1/orders/{id}"));
        assert!(parse_operation("kafka_produce:orders.paid").is_none());
        assert!(parse_operation("http:orders:GET").is_none());
        assert!(parse_operation("http:orders:GET ").is_none());
    }

    #[tokio::test]
    async fn post_json_body_then_extract() {
        let base = stub().await;
        let ex = executor(&base);
        let run = run_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        ctx.begin_step("create");
        let mut http = http_step("http:orders:POST /v1/orders");
        http.body = Some(
            json!({"customerId": "{{users.id}}", "items": [{"sku": "{{pick skus.sku}}", "qty": "{{int 2 2}}"}]}),
        );
        let expect = Expect {
            status: Some(StatusExpect::One(201)),
            jsonpath: BTreeMap::from([("$.id".to_string(), Matcher::Exists(ExistsTag::Exists))]),
            schema: None,
            max_ms: Some(2000),
        };
        let out = ex
            .execute(&step(http, expect, &[("order_id", "$.id")]), &mut ctx)
            .await
            .unwrap();
        assert_eq!(out.status, Some(201));
        let body = out.body.unwrap();
        assert_eq!(body["id"], json!("A-1"));
        assert_eq!(
            body["echo"],
            json!({"customerId": "7", "items": [{"sku": "S1", "qty": 2}]}),
            "одно выражение сохраняет тип"
        );
        assert_eq!(
            out.bytes_out,
            serde_json::to_vec(&body["echo"]).unwrap().len() as u64
        );
        assert!(out.bytes_in > 0 && out.latency_us > 0);
        assert_eq!(ctx.step("create", "order_id"), Some(json!("A-1")));
    }

    #[tokio::test]
    async fn http_errors_carry_masked_samples() {
        let base = stub().await;
        let ex = executor(&base);
        let run = run_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        let mut http = http_step("http:orders:GET /fail");
        http.headers
            .insert("Authorization".into(), "Bearer {{secret API_TOKEN}}".into());
        let e = ex
            .execute(&step(http, Expect::default(), &[]), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (StepCode::Http5xx, "status 500")
        );
        let req = e.request.unwrap();
        assert!(req.starts_with(&format!("GET {base}/fail\n")), "{req}");
        assert!(req.contains("authorization: Bearer ***"), "{req}");
        assert!(req.contains("accept: application/json"), "{req}");
        assert!(!req.contains("s3cr3t"));
        let resp = e.response.unwrap();
        assert!(
            resp.starts_with("500\n") && resp.ends_with("\n\nboom"),
            "{resp}"
        );

        let e = ex
            .execute(&get_step("/teapot"), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (StepCode::Http4xx, "status 418")
        );
    }

    #[tokio::test]
    async fn expect_status_overrides_http_codes() {
        let base = stub().await;
        let ex = executor(&base);
        let run = run_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        let ok = step(
            http_step("http:orders:GET /fail"),
            Expect {
                status: Some(StatusExpect::One(500)),
                ..Default::default()
            },
            &[],
        );
        assert_eq!(ex.execute(&ok, &mut ctx).await.unwrap().status, Some(500));
        let class = step(
            http_step("http:orders:GET /teapot"),
            Expect {
                status: Some(StatusExpect::Class("2xx".into())),
                ..Default::default()
            },
            &[],
        );
        let e = ex.execute(&class, &mut ctx).await.unwrap_err();
        assert_eq!(e.code, StepCode::ExpectStatus);
        assert_eq!(e.message, "expected status 2xx, got 418");
        assert!(e.request.is_some() && e.response.is_some());
        let out = ex.execute(&get_step("/redirect"), &mut ctx).await.unwrap();
        assert_eq!(out.status, Some(302), "редирект отдаётся как есть");
        let out = ex.execute(&get_step("/text"), &mut ctx).await.unwrap();
        assert_eq!((out.body, out.bytes_in), (None, 5));
    }

    #[tokio::test]
    async fn timeout_and_connect_errors() {
        let base = stub().await;
        let ex = executor(&base);
        let run = run_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        let mut http = http_step("http:orders:GET /slow");
        http.timeout_ms = Some(200);
        let started = Instant::now();
        let e = ex
            .execute(&step(http, Expect::default(), &[]), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(
            (e.code, e.message.as_str()),
            (StepCode::Timeout, "no response in 200 ms")
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(e.request.is_some() && e.response.is_none());

        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = free.local_addr().unwrap();
        drop(free);
        let ex = executor(&format!("http://{addr}"));
        let e = ex.execute(&get_step("/x"), &mut ctx).await.unwrap_err();
        // На Windows подключение к закрытому порту не отклоняется сразу, а висит до таймаута запроса
        // (так ведёт себя раннер GitHub), поэтому ошибка классифицируется как Timeout.
        if cfg!(windows) {
            assert!(
                matches!(e.code, StepCode::Connect | StepCode::Timeout),
                "{:?}",
                e.code
            );
        } else {
            assert_eq!(e.code, StepCode::Connect);
        }
    }

    #[tokio::test]
    async fn path_query_and_headers_are_rendered() {
        let base = stub().await;
        let ex = executor(&base);
        let run = run_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        ctx.begin_step("create");
        ctx.store("order_id", json!("A 1/2"));
        ctx.begin_step("get");
        let mut http = http_step("http:orders:GET /v1/orders/{id}");
        http.path
            .insert("id".into(), "{{steps.create.order_id}}".into());
        http.query.insert("expand".into(), "items".into());
        http.query.insert("user".into(), "{{users.id}}".into());
        http.headers
            .insert("X-Run".into(), "{{run.id}}/{{agent.id}}".into());
        let out = ex
            .execute(&step(http, Expect::default(), &[]), &mut ctx)
            .await
            .unwrap();
        let body = out.body.unwrap();
        assert_eq!(
            body["id"],
            json!("A 1/2"),
            "сегмент пути закодирован и раскодирован сервером"
        );
        assert_eq!(body["query"], json!({"expand": "items", "user": "7"}));
        assert_eq!(body["headers"]["x-run"], json!("run-1/gen-1"));
        assert_eq!(
            body["headers"]["accept"],
            json!("application/json"),
            "заголовок по умолчанию"
        );

        let mut missing = http_step("http:orders:GET /v1/orders/{id}");
        missing.path.insert("other".into(), "1".into());
        let e = ex
            .execute(&step(missing, Expect::default(), &[]), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(e.code, StepCode::Connect);
        assert!(
            e.message.contains("path parameter `id` is not set"),
            "{}",
            e.message
        );
        let mut unknown = http_step("http:orders:GET /v1/orders/{id}");
        unknown.path.insert("id".into(), "{{steps.nope.x}}".into());
        let e = ex
            .execute(&step(unknown, Expect::default(), &[]), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(e.code, StepCode::ExtractMissing);
        assert!(e.message.starts_with("expression:"), "{}", e.message);
    }

    #[tokio::test]
    async fn jsonpath_schema_max_ms_and_extract_missing() {
        let base = stub().await;
        let schema = SchemaSummary::object(
            "body",
            vec![SchemaSummary::leaf("id", "string").required(true)],
        );
        let strict = SchemaSummary::object(
            "body",
            vec![SchemaSummary::leaf("missing", "string").required(true)],
        );
        let op = "http:orders:GET /v1/orders/{id}";
        let run = run_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        let mut http = http_step(op);
        http.path.insert("id".into(), "A-1".into());

        let ex = executor(&base).with_schemas(BTreeMap::from([(op.to_string(), schema)]));
        let good = Expect {
            jsonpath: BTreeMap::from([(
                "$.id".to_string(),
                Matcher::Equals {
                    equals: json!("A-1"),
                },
            )]),
            schema: Some(SchemaExpect::Response),
            ..Default::default()
        };
        assert!(
            ex.execute(&step(http.clone(), good, &[]), &mut ctx)
                .await
                .is_ok()
        );
        let bad = Expect {
            jsonpath: BTreeMap::from([(
                "$.id".to_string(),
                Matcher::Equals { equals: json!("B") },
            )]),
            ..Default::default()
        };
        let e = ex
            .execute(&step(http.clone(), bad, &[]), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(e.code, StepCode::ExpectJsonpath);
        assert!(e.response.unwrap().starts_with("200\n"));
        let slow = Expect {
            max_ms: Some(0),
            ..Default::default()
        };
        let e = ex
            .execute(&step(http.clone(), slow, &[]), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(e.code, StepCode::ExpectMaxMs);

        let ex = executor(&base).with_schemas(BTreeMap::from([(op.to_string(), strict)]));
        let schema_expect = Expect {
            schema: Some(SchemaExpect::Response),
            ..Default::default()
        };
        let e = ex
            .execute(&step(http.clone(), schema_expect, &[]), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(e.code, StepCode::ExpectSchema);
        assert!(
            e.message.contains("$.missing: required field is missing"),
            "{}",
            e.message
        );

        let e = ex
            .execute(&step(http, Expect::default(), &[("x", "$.nope")]), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(e.code, StepCode::ExtractMissing);
        assert!(e.request.is_some() && e.response.is_some());
    }

    #[tokio::test]
    async fn body_text_is_sent_as_is() {
        let base = stub().await;
        let ex = executor(&base);
        let run = run_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        let mut http = http_step("http:orders:POST /v1/orders");
        http.body_text = Some("{\"raw\": \"{{users.id}}\"}".into());
        http.headers
            .insert("Content-Type".into(), "application/json".into());
        let out = ex
            .execute(&step(http, Expect::default(), &[]), &mut ctx)
            .await
            .unwrap();
        assert_eq!(out.body.unwrap()["echo"], json!({"raw": "7"}));
        assert_eq!(out.bytes_out, 12);
    }

    /// Секрет со спецсимволами: в query и в параметре пути он попадает в URL в закодированном виде.
    const TRICKY: &str = "a+b/c=d e";

    fn tricky_ctx() -> RunContext {
        let mut run = run_ctx();
        run.secrets.insert("TRICKY".to_string(), TRICKY.to_string());
        run
    }

    fn tricky_executor(base: &str) -> HttpExecutor {
        let defaults = HttpDefaults {
            timeout_ms: 2000,
            headers: BTreeMap::new(),
        };
        HttpExecutor::new(base, &defaults, &[TRICKY.to_string()]).unwrap()
    }

    fn tricky_step(path: &str) -> Step {
        let mut http = http_step(&format!("http:orders:GET {path}"));
        http.path.insert("id".into(), "{{secret TRICKY}}".into());
        http.query.insert("q".into(), "{{secret TRICKY}}".into());
        step(http, Expect::default(), &[])
    }

    /// Ни исходный секрет, ни его закодированные формы не должны остаться в тексте.
    fn assert_no_secret(text: &str) {
        for form in secret_variants(&[TRICKY.to_string()]) {
            assert!(!text.contains(&form), "найдено `{form}` в: {text}");
        }
    }

    #[test]
    fn secret_variants_cover_encodings() {
        let v = secret_variants(&[TRICKY.to_string(), String::new(), TRICKY.to_string()]);
        assert!(v.contains(&TRICKY.to_string()));
        assert!(v.contains(&"a%2Bb%2Fc%3Dd%20e".to_string()), "{v:?}");
        assert!(v.contains(&"a%2Bb%2Fc%3Dd+e".to_string()), "{v:?}");
        let mut dedup = v.clone();
        dedup.sort();
        dedup.dedup();
        assert_eq!(dedup.len(), v.len(), "без дубликатов");
        assert!(secret_variants(&[String::new()]).is_empty());
    }

    #[tokio::test]
    async fn encoded_secrets_are_masked_in_url_samples() {
        let base = stub().await;
        let ex = tricky_executor(&base);
        let run = tricky_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        let e = ex
            .execute(&tricky_step("/v1/fail/{id}"), &mut ctx)
            .await
            .unwrap_err();
        assert_eq!(e.code, StepCode::Http5xx);
        let req = e.request.unwrap();
        assert!(req.contains("***"), "{req}");
        assert_no_secret(&req);
        assert_no_secret(&e.response.unwrap());
        assert_no_secret(&e.message);

        // Ошибка отправки: reqwest вставляет URL в сообщение.
        let free = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = free.local_addr().unwrap();
        drop(free);
        let ex = tricky_executor(&format!("http://{addr}"));
        let e = ex
            .execute(&tricky_step("/v1/fail/{id}"), &mut ctx)
            .await
            .unwrap_err();
        // Windows: подключение к закрытому порту висит до таймаута и даёт Timeout; его сообщение
        // не содержит URL, поэтому маскировать нечего.
        if cfg!(windows) {
            assert!(
                matches!(e.code, StepCode::Connect | StepCode::Timeout),
                "{:?}",
                e.code
            );
        } else {
            assert_eq!(e.code, StepCode::Connect);
        }
        if e.code == StepCode::Connect {
            assert!(e.message.contains("***"), "{}", e.message);
        }
        assert_no_secret(&e.message);
        assert_no_secret(&e.request.unwrap());
    }

    #[tokio::test]
    async fn body_read_failure_keeps_masked_status_and_headers() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            let mut buf = [0u8; 4096];
            let _ = conn.read(&mut buf);
            let _ = conn.write_all(
                b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\nX-Token: s3cr3t\r\n\r\npartial",
            );
        });
        let ex = executor(&format!("http://{addr}"));
        let run = run_ctx();
        let mut ctx = IterationCtx::new(&run, 0);
        let e = ex.execute(&get_step("/cut"), &mut ctx).await.unwrap_err();
        assert_eq!(e.code, StepCode::Connect);
        assert!(e.request.is_some());
        let resp = e.response.expect("пример ответа сохраняется");
        assert!(resp.starts_with("200\n"), "{resp}");
        assert!(resp.contains("x-token: ***"), "{resp}");
        assert!(!resp.contains("s3cr3t"), "{resp}");
    }

    #[test]
    fn base_url_is_validated() {
        let defaults = HttpDefaults {
            timeout_ms: 1000,
            headers: BTreeMap::new(),
        };
        for bad in ["", "not a url", "localhost:8080", "ftp://host", "http://"] {
            let e = HttpExecutor::new(bad, &defaults, &[])
                .err()
                .unwrap_or_else(|| panic!("`{bad}` должен быть отвергнут"));
            assert!(e.starts_with("invalid base url"), "{e}");
        }
        let ok = HttpExecutor::new("http://127.0.0.1:8080/", &defaults, &[]).unwrap();
        assert_eq!(ok.base_url(), "http://127.0.0.1:8080");
    }
}
