//! Выражения `{{…}}` в строках сьюта: разбор, обход сьюта и подстановка (спека L2 §3.3).
//! Формы ровно из спеки, без операторов и вложенности. Случайность и время приходят из
//! `Context`: модель не зависит от ОС и воспроизводима в тестах.

use std::collections::BTreeMap;

use crate::suite::schema::{Action, Step, Suite};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NowFmt {
    /// RFC 3339 в UTC.
    Iso,
    /// Секунды с эпохи.
    Unix,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    /// `users.id` — колонка текущей строки набора.
    Data {
        set: String,
        col: String,
    },
    /// `steps.create.order_id` — значение `extract` предыдущего шага.
    Step {
        step: String,
        name: String,
    },
    Uuid,
    /// `int a b` — целое в `[a, b]`.
    Int(i64, i64),
    /// `float a b` — вещественное в `[a, b]`.
    Float(f64, f64),
    Now(NowFmt),
    /// `str n` — `n` латинских букв и цифр.
    Str(usize),
    /// `pick skus.sku` — колонка случайной строки набора.
    Pick {
        set: String,
        col: String,
    },
    /// `secret API_TOKEN` — переменная `GLAFIRA_LOAD_SECRET_API_TOKEN` агента.
    Secret(String),
    /// `env.http`, `env.kafka`, `env.schema_registry` — адреса стенда.
    Env(String),
    RunId,
    AgentId,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    Expr(Expr),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Template(pub Vec<Part>);

impl Template {
    /// Строка целиком из одного выражения: значение сохраняет тип.
    pub fn single(&self) -> Option<&Expr> {
        match self.0.as_slice() {
            [Part::Expr(e)] => Some(e),
            _ => None,
        }
    }

    pub fn exprs(&self) -> impl Iterator<Item = &Expr> {
        self.0.iter().filter_map(|p| match p {
            Part::Expr(e) => Some(e),
            Part::Text(_) => None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ExprError {
    pub message: String,
}

fn err(message: impl Into<String>) -> ExprError {
    ExprError {
        message: message.into(),
    }
}

/// Сегмент пути ключа в сьюте: `scenarios[0].steps[1].http.body`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PathSeg {
    Key(String),
    Index(usize),
}

/// Адреса стенда, доступные как `env.<ключ>`.
pub const ENV_KEYS: [&str; 3] = ["http", "kafka", "schema_registry"];
/// Верхняя граница `str n`.
pub const MAX_STR_LEN: usize = 1024;

/// Разбор строки с `{{…}}`; строка без выражений — один `Part::Text`, пустая — без частей.
pub fn parse_template(text: &str) -> Result<Template, ExprError> {
    let mut parts = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{") {
        if start > 0 {
            parts.push(Part::Text(rest[..start].to_string()));
        }
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(err(format!("unclosed `{{{{` in `{text}`")));
        };
        parts.push(Part::Expr(parse_expr(after[..end].trim())?));
        rest = &after[end + 2..];
    }
    if !rest.is_empty() {
        parts.push(Part::Text(rest.to_string()));
    }
    Ok(Template(parts))
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

fn set_col(form: &str, arg: &str) -> Result<(String, String), ExprError> {
    match arg.split_once('.') {
        Some((set, col)) if is_name(set) && is_name(col) => Ok((set.into(), col.into())),
        _ => Err(err(format!(
            "`{form}` expects `<set>.<column>`, got `{arg}`"
        ))),
    }
}

fn int_arg(form: &str, s: &str) -> Result<i64, ExprError> {
    s.parse().map_err(|_| {
        err(format!(
            "`{form}` takes two integer arguments: `{form} <min> <max>`, got `{s}`"
        ))
    })
}

fn float_arg(form: &str, s: &str) -> Result<f64, ExprError> {
    match s.parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => Err(err(format!(
            "`{form}` takes two numbers: `{form} <min> <max>`, got `{s}`"
        ))),
    }
}

fn parse_expr(src: &str) -> Result<Expr, ExprError> {
    let tokens: Vec<&str> = src.split_whitespace().collect();
    match tokens.as_slice() {
        [] => Err(err("empty expression `{{}}`")),
        ["uuid"] => Ok(Expr::Uuid),
        ["run.id"] => Ok(Expr::RunId),
        ["agent.id"] => Ok(Expr::AgentId),
        ["int", a, b] => {
            let (a, b) = (int_arg("int", a)?, int_arg("int", b)?);
            if a > b {
                return Err(err(format!("`int {a} {b}`: min is greater than max")));
            }
            Ok(Expr::Int(a, b))
        }
        ["int", ..] => Err(err("`int` takes two integer arguments: `int <min> <max>`")),
        ["float", a, b] => {
            let (a, b) = (float_arg("float", a)?, float_arg("float", b)?);
            if a > b {
                return Err(err(format!("`float {a} {b}`: min is greater than max")));
            }
            Ok(Expr::Float(a, b))
        }
        ["float", ..] => Err(err("`float` takes two numbers: `float <min> <max>`")),
        ["now", "iso"] => Ok(Expr::Now(NowFmt::Iso)),
        ["now", "unix"] => Ok(Expr::Now(NowFmt::Unix)),
        ["now", ..] => Err(err("`now` takes `iso` or `unix`")),
        ["str", n] => match n.parse::<usize>() {
            Ok(n) if (1..=MAX_STR_LEN).contains(&n) => Ok(Expr::Str(n)),
            _ => Err(err(format!(
                "`str` takes a length from 1 to {MAX_STR_LEN}, got `{n}`"
            ))),
        },
        ["str", ..] => Err(err("`str` takes one argument: `str <length>`")),
        ["pick", arg] => {
            let (set, col) = set_col("pick", arg)?;
            Ok(Expr::Pick { set, col })
        }
        ["pick", ..] => Err(err("`pick` takes one argument: `pick <set>.<column>`")),
        ["secret", name] if is_name(name) => Ok(Expr::Secret(name.to_string())),
        ["secret", ..] => Err(err(
            "`secret` takes one name of letters, digits and `_`: `secret <NAME>`",
        )),
        [one] if one.starts_with("steps.") => {
            let p: Vec<&str> = one.split('.').collect();
            match p.as_slice() {
                ["steps", step, name] if is_name(step) && is_name(name) => Ok(Expr::Step {
                    step: step.to_string(),
                    name: name.to_string(),
                }),
                _ => Err(err(format!("`{one}`: expected `steps.<step>.<name>`"))),
            }
        }
        [one] if one.starts_with("env.") => {
            let key = &one[4..];
            if ENV_KEYS.contains(&key) {
                Ok(Expr::Env(key.to_string()))
            } else {
                Err(err(format!(
                    "unknown environment address `{key}`; expected env.http, env.kafka or env.schema_registry"
                )))
            }
        }
        [one] if one.contains('.') && !one.starts_with("run.") && !one.starts_with("agent.") => {
            let (set, col) = set_col("data", one)?;
            Ok(Expr::Data { set, col })
        }
        _ => Err(err(format!("unknown expression `{src}`"))),
    }
}

fn push_str(out: &mut Vec<(Vec<PathSeg>, String)>, path: Vec<PathSeg>, s: &str) {
    if s.contains("{{") {
        out.push((path, s.to_string()));
    }
}

fn push_opt(out: &mut Vec<(Vec<PathSeg>, String)>, path: Vec<PathSeg>, s: &Option<String>) {
    if let Some(s) = s {
        push_str(out, path, s);
    }
}

fn push_map(
    out: &mut Vec<(Vec<PathSeg>, String)>,
    prefix: &[PathSeg],
    key: &str,
    map: &BTreeMap<String, String>,
) {
    for (k, v) in map {
        let mut path = prefix.to_vec();
        path.push(PathSeg::Key(key.into()));
        path.push(PathSeg::Key(k.clone()));
        push_str(out, path, v);
    }
}

fn push_json(out: &mut Vec<(Vec<PathSeg>, String)>, path: Vec<PathSeg>, v: &serde_json::Value) {
    match v {
        serde_json::Value::String(s) => push_str(out, path, s),
        serde_json::Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                let mut p = path.clone();
                p.push(PathSeg::Index(i));
                push_json(out, p, item);
            }
        }
        serde_json::Value::Object(map) => {
            for (k, item) in map {
                let mut p = path.clone();
                p.push(PathSeg::Key(k.clone()));
                push_json(out, p, item);
            }
        }
        _ => {}
    }
}

fn with(prefix: &[PathSeg], keys: &[&str]) -> Vec<PathSeg> {
    let mut p = prefix.to_vec();
    p.extend(keys.iter().map(|k| PathSeg::Key((*k).into())));
    p
}

/// Строки шага с `{{`: заголовки, параметры пути и запроса, тело (рекурсивно по JSON), ключ и
/// значение сообщения, `match`. `extract` и `expect.jsonpath` — JSONPath, не шаблоны.
pub fn step_templates(prefix: &[PathSeg], step: &Step) -> Vec<(Vec<PathSeg>, String)> {
    let mut out = Vec::new();
    match &step.action {
        Action::Http(h) => {
            let p = with(prefix, &["http"]);
            push_map(&mut out, &p, "path", &h.path);
            push_map(&mut out, &p, "query", &h.query);
            push_map(&mut out, &p, "headers", &h.headers);
            if let Some(body) = &h.body {
                push_json(&mut out, with(&p, &["body"]), body);
            }
            push_opt(&mut out, with(&p, &["body_text"]), &h.body_text);
        }
        Action::KafkaProduce(k) => {
            let p = with(prefix, &["kafka_produce"]);
            push_opt(&mut out, with(&p, &["key"]), &k.key);
            if let Some(value) = &k.value {
                push_json(&mut out, with(&p, &["value"]), value);
            }
            push_opt(&mut out, with(&p, &["value_text"]), &k.value_text);
            push_map(&mut out, &p, "headers", &k.headers);
        }
        Action::KafkaConsume(c) => {
            let p = with(prefix, &["kafka_consume", "match"]);
            push_opt(&mut out, with(&p, &["key"]), &c.match_.key);
            push_map(&mut out, &p, "jsonpath", &c.match_.jsonpath);
        }
    }
    out
}

/// Все строки сьюта с `{{` и путь ключа каждой: `defaults.http.headers` и шаги всех сценариев.
pub fn templates_of(suite: &Suite) -> Vec<(Vec<PathSeg>, String)> {
    let mut out = Vec::new();
    push_map(
        &mut out,
        &with(&[], &["defaults", "http"]),
        "headers",
        &suite.defaults.http.headers,
    );
    for (i, sc) in suite.scenarios.iter().enumerate() {
        for (j, step) in sc.steps.iter().enumerate() {
            let prefix = [
                PathSeg::Key("scenarios".into()),
                PathSeg::Index(i),
                PathSeg::Key("steps".into()),
                PathSeg::Index(j),
            ];
            out.extend(step_templates(&prefix, step));
        }
    }
    out
}

/// Источник значений для подстановки: текущие строки наборов, `extract` итерации, секреты,
/// адреса стенда, идентификаторы, случайность и часы.
pub trait Context {
    fn data(&self, set: &str, col: &str) -> Option<serde_json::Value>;
    fn pick(&self, set: &str, col: &str) -> Option<serde_json::Value>;
    fn step(&self, step: &str, name: &str) -> Option<serde_json::Value>;
    fn secret(&self, name: &str) -> Option<String>;
    fn env(&self, key: &str) -> Option<String>;
    fn run_id(&self) -> &str;
    fn agent_id(&self) -> &str;
    /// Источник случайности.
    fn random_u64(&self) -> u64;
    fn now_unix_ms(&self) -> i64;
}

const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";

/// UUID v4 из 128 случайных бит: биты версии и варианта по RFC 9562.
pub fn uuid_v4(hi: u64, lo: u64) -> String {
    let mut b = [0u8; 16];
    b[..8].copy_from_slice(&hi.to_be_bytes());
    b[8..].copy_from_slice(&lo.to_be_bytes());
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}

/// RFC 3339 в UTC из миллисекунд с эпохи.
pub fn rfc3339_ms(ms: i64) -> Result<String, ExprError> {
    time::OffsetDateTime::from_unix_timestamp_nanos(ms as i128 * 1_000_000)
        .ok()
        .and_then(|t| {
            t.format(&time::format_description::well_known::Rfc3339)
                .ok()
        })
        .ok_or_else(|| err(format!("time {ms} ms is out of range")))
}

fn eval(e: &Expr, ctx: &dyn Context) -> Result<serde_json::Value, ExprError> {
    use serde_json::Value;
    Ok(match e {
        Expr::Data { set, col } => ctx
            .data(set, col)
            .ok_or_else(|| err(format!("no value for `{set}.{col}`")))?,
        Expr::Pick { set, col } => ctx
            .pick(set, col)
            .ok_or_else(|| err(format!("no value for `pick {set}.{col}`")))?,
        Expr::Step { step, name } => ctx
            .step(step, name)
            .ok_or_else(|| err(format!("no value for `steps.{step}.{name}`")))?,
        Expr::Uuid => Value::String(uuid_v4(ctx.random_u64(), ctx.random_u64())),
        Expr::Int(a, b) => {
            let span = (*b as i128 - *a as i128 + 1) as u128;
            Value::from((*a as i128 + (ctx.random_u64() as u128 % span) as i128) as i64)
        }
        Expr::Float(a, b) => {
            let u = ctx.random_u64() as f64 / u64::MAX as f64;
            Value::from(a + (b - a) * u)
        }
        Expr::Now(NowFmt::Iso) => Value::String(rfc3339_ms(ctx.now_unix_ms())?),
        Expr::Now(NowFmt::Unix) => Value::from(ctx.now_unix_ms().div_euclid(1000)),
        Expr::Str(n) => Value::String(
            (0..*n)
                .map(|_| ALPHABET[(ctx.random_u64() % ALPHABET.len() as u64) as usize] as char)
                .collect(),
        ),
        Expr::Secret(name) => Value::String(
            ctx.secret(name)
                .ok_or_else(|| err(format!("secret `{name}` is not set")))?,
        ),
        Expr::Env(key) => Value::String(
            ctx.env(key)
                .ok_or_else(|| err(format!("environment has no `{key}` address")))?,
        ),
        Expr::RunId => Value::String(ctx.run_id().to_string()),
        Expr::AgentId => Value::String(ctx.agent_id().to_string()),
    })
}

fn to_text(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => String::new(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        other => other.to_string(),
    }
}

/// Значение шаблона: одно выражение сохраняет тип, иначе строка.
pub fn render(t: &Template, ctx: &dyn Context) -> Result<serde_json::Value, ExprError> {
    match t.single() {
        Some(e) => eval(e, ctx),
        None => Ok(serde_json::Value::String(render_string(t, ctx)?)),
    }
}

pub fn render_string(t: &Template, ctx: &dyn Context) -> Result<String, ExprError> {
    let mut out = String::new();
    for part in &t.0 {
        match part {
            Part::Text(s) => out.push_str(s),
            Part::Expr(e) => out.push_str(&to_text(&eval(e, ctx)?)),
        }
    }
    Ok(out)
}

/// Подстановка во всех строках JSON-значения (тело запроса, значение сообщения); ключи объектов
/// не трогаются.
pub fn render_json(
    v: &serde_json::Value,
    ctx: &dyn Context,
) -> Result<serde_json::Value, ExprError> {
    use serde_json::Value;
    Ok(match v {
        Value::String(s) if s.contains("{{") => render(&parse_template(s)?, ctx)?,
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|i| render_json(i, ctx))
                .collect::<Result<_, _>>()?,
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, i)| Ok((k.clone(), render_json(i, ctx)?)))
                .collect::<Result<_, ExprError>>()?,
        ),
        other => other.clone(),
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::cell::Cell;

    /// Контекст для тестов: таблицы и детерминированный xorshift.
    pub(crate) struct TestCtx {
        pub data: BTreeMap<String, Value>,
        pub steps: BTreeMap<String, Value>,
        pub secrets: BTreeMap<String, String>,
        pub env: BTreeMap<String, String>,
        pub seed: Cell<u64>,
        pub now_ms: i64,
    }

    impl Default for TestCtx {
        fn default() -> Self {
            Self {
                data: [
                    ("users.id", json!("u-1001")),
                    ("users.age", json!(42)),
                    ("skus.sku", json!("SKU-100")),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                steps: [
                    ("create.order_id", json!("o-7")),
                    ("create.total", json!(12.5)),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
                secrets: [("API_TOKEN", "t0k")]
                    .into_iter()
                    .map(|(k, v)| (k.to_string(), v.to_string()))
                    .collect(),
                env: [
                    ("http", "https://orders.test.local"),
                    ("kafka", "kafka-1:9092"),
                ]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
                seed: Cell::new(0x9E37_79B9_7F4A_7C15),
                now_ms: 1_793_530_800_123, // 2026-11-01T11:00:00.123Z
            }
        }
    }

    impl Context for TestCtx {
        fn data(&self, set: &str, col: &str) -> Option<Value> {
            self.data.get(&format!("{set}.{col}")).cloned()
        }
        fn pick(&self, set: &str, col: &str) -> Option<Value> {
            self.data(set, col)
        }
        fn step(&self, step: &str, name: &str) -> Option<Value> {
            self.steps.get(&format!("{step}.{name}")).cloned()
        }
        fn secret(&self, name: &str) -> Option<String> {
            self.secrets.get(name).cloned()
        }
        fn env(&self, key: &str) -> Option<String> {
            self.env.get(key).cloned()
        }
        fn run_id(&self) -> &str {
            "run-1"
        }
        fn agent_id(&self) -> &str {
            "gen-1"
        }
        fn random_u64(&self) -> u64 {
            let mut x = self.seed.get();
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.seed.set(x);
            x
        }
        fn now_unix_ms(&self) -> i64 {
            self.now_ms
        }
    }

    fn value(text: &str) -> Value {
        render(&parse_template(text).unwrap(), &TestCtx::default()).unwrap()
    }

    #[test]
    fn table_string_to_value_keeps_the_type_of_a_single_expression() {
        let cases: Vec<(&str, Value)> = vec![
            ("{{users.id}}", json!("u-1001")),
            ("{{users.age}}", json!(42)),
            ("{{ users.age }}", json!(42)),
            ("id-{{users.id}}", json!("id-u-1001")),
            ("{{users.age}} years", json!("42 years")),
            ("{{steps.create.order_id}}", json!("o-7")),
            ("{{steps.create.total}}", json!(12.5)),
            ("{{int 1 1}}", json!(1)),
            ("id-{{int 1 1}}", json!("id-1")),
            ("{{float 2.5 2.5}}", json!(2.5)),
            ("{{now unix}}", json!(1_793_530_800)),
            ("{{now iso}}", json!("2026-11-01T11:00:00.123Z")),
            ("{{pick skus.sku}}", json!("SKU-100")),
            ("Bearer {{secret API_TOKEN}}", json!("Bearer t0k")),
            ("{{env.http}}/v1", json!("https://orders.test.local/v1")),
            ("{{run.id}}/{{agent.id}}", json!("run-1/gen-1")),
            ("plain", json!("plain")),
            ("", json!("")),
            ("a }} b", json!("a }} b")),
        ];
        for (text, expected) in cases {
            assert_eq!(value(text), expected, "{text}");
        }
    }

    #[test]
    fn random_forms_stay_in_range_and_have_the_right_shape() {
        let ctx = TestCtx::default();
        for _ in 0..200 {
            let v = render(&parse_template("{{int -3 2}}").unwrap(), &ctx).unwrap();
            assert!((-3..=2).contains(&v.as_i64().unwrap()), "{v}");
            let f = render(&parse_template("{{float 0.5 1.5}}").unwrap(), &ctx)
                .unwrap()
                .as_f64()
                .unwrap();
            assert!((0.5..=1.5).contains(&f), "{f}");
            let s = render_string(&parse_template("{{str 8}}").unwrap(), &ctx).unwrap();
            assert!(
                s.len() == 8 && s.bytes().all(|b| b.is_ascii_alphanumeric()),
                "{s}"
            );
            let u = render_string(&parse_template("{{uuid}}").unwrap(), &ctx).unwrap();
            assert_eq!(u.len(), 36);
            assert_eq!((&u[14..15], &u[8..9]), ("4", "-"), "{u}");
            assert!(matches!(&u[19..20], "8" | "9" | "a" | "b"), "{u}");
        }
        assert_eq!(
            value("{{int 9223372036854775807 9223372036854775807}}"),
            json!(i64::MAX),
            "без переполнения"
        );
        assert_eq!(uuid_v4(0, 0), "00000000-0000-4000-8000-000000000000");
    }

    #[test]
    fn parse_errors_name_the_form() {
        let bad = [
            ("{{frob}}", "unknown expression `frob`"),
            ("{{int 1}}", "`int` takes two integer arguments"),
            ("{{int 1 2 3}}", "`int` takes two integer arguments"),
            ("{{int a b}}", "`int` takes two integer arguments"),
            ("{{int 5 1}}", "min is greater than max"),
            ("{{float 1 x}}", "`float` takes two numbers"),
            ("{{now}}", "`now` takes `iso` or `unix`"),
            ("{{now utc}}", "`now` takes `iso` or `unix`"),
            ("{{str}}", "`str` takes one argument"),
            ("{{str 0}}", "`str` takes a length"),
            ("{{pick skus}}", "`pick` expects `<set>.<column>`"),
            ("{{secret}}", "`secret` takes one name"),
            ("{{secret a b}}", "`secret` takes one name"),
            ("{{secret a.b}}", "`secret` takes one name"),
            ("{{steps.create}}", "expected `steps.<step>.<name>`"),
            ("{{steps.a.b.c}}", "expected `steps.<step>.<name>`"),
            ("{{env.grpc}}", "unknown environment address `grpc`"),
            ("{{run.name}}", "unknown expression `run.name`"),
            ("{{a.b.c}}", "`data` expects `<set>.<column>`"),
            ("{{users.id", "unclosed `{{`"),
            ("x {{", "unclosed `{{`"),
            ("{{}}", "empty expression"),
            ("{{ uuid now }}", "unknown expression `uuid now`"),
        ];
        for (text, expected) in bad {
            let e = parse_template(text).unwrap_err();
            assert!(e.message.contains(expected), "{text}: {}", e.message);
        }
    }

    #[test]
    fn parse_template_splits_text_and_expressions() {
        let t = parse_template("a{{uuid}}b{{int 1 2}}").unwrap();
        assert_eq!(
            t.0,
            vec![
                Part::Text("a".into()),
                Part::Expr(Expr::Uuid),
                Part::Text("b".into()),
                Part::Expr(Expr::Int(1, 2))
            ]
        );
        assert!(t.single().is_none());
        assert_eq!(t.exprs().count(), 2);
        assert_eq!(
            parse_template("{{users.id}}").unwrap().single(),
            Some(&Expr::Data {
                set: "users".into(),
                col: "id".into()
            })
        );
        assert_eq!(parse_template("").unwrap().0, vec![]);
    }

    #[test]
    fn render_errors_for_missing_values() {
        let ctx = TestCtx::default();
        for (text, expected) in [
            ("{{users.phone}}", "no value for `users.phone`"),
            ("{{steps.get.x}}", "no value for `steps.get.x`"),
            ("{{secret OTHER}}", "secret `OTHER` is not set"),
            (
                "{{env.schema_registry}}",
                "environment has no `schema_registry` address",
            ),
        ] {
            let e = render(&parse_template(text).unwrap(), &ctx).unwrap_err();
            assert_eq!(e.message, expected);
        }
    }

    #[test]
    fn render_json_walks_values_and_keeps_keys() {
        let ctx = TestCtx::default();
        let body = json!({ "customerId": "{{users.id}}", "items": [{ "sku": "{{pick skus.sku}}", "qty": "{{int 2 2}}" }], "n": 1, "{{users.id}}": true });
        let out = render_json(&body, &ctx).unwrap();
        assert_eq!(
            out,
            json!({ "customerId": "u-1001", "items": [{ "sku": "SKU-100", "qty": 2 }], "n": 1, "{{users.id}}": true })
        );
        assert!(render_json(&json!({ "a": "{{nope}}" }), &ctx).is_err());
    }

    #[test]
    fn templates_of_walks_every_string_field_with_a_path() {
        let suite = crate::suite::parse(include_str!(
            "../../../../dev/fixtures/suites/orders/suite.yaml"
        ))
        .unwrap();
        let mut found: Vec<String> = templates_of(&suite)
            .iter()
            .map(|(p, s)| format!("{} = {s}", crate::suite::lines::path_string(p)))
            .collect();
        // Порядок ключей JSON-тела зависит от фичи serde_json/preserve_order
        // (в workspace её включает glafira-load-contracts), поэтому сравниваем
        // отсортированные списки: состав и количество записей проверяются полностью.
        found.sort();
        let mut expected = vec![
            "scenarios[0].steps[0].http.headers.Authorization = Bearer {{secret API_TOKEN}}",
            "scenarios[0].steps[0].http.body.customerId = {{users.id}}",
            "scenarios[0].steps[0].http.body.items[0].qty = {{int 1 5}}",
            "scenarios[0].steps[0].http.body.items[0].sku = {{pick skus.sku}}",
            "scenarios[0].steps[1].http.path.id = {{steps.create.order_id}}",
            "scenarios[0].steps[2].kafka_consume.match.key = {{steps.create.order_id}}",
            "scenarios[0].steps[3].kafka_produce.key = {{steps.create.order_id}}",
            "scenarios[0].steps[3].kafka_produce.value.orderId = {{steps.create.order_id}}",
            "scenarios[0].steps[3].kafka_produce.value.paidAt = {{now iso}}",
        ];
        expected.sort();
        assert_eq!(found, expected);
        let mut s2 = suite.clone();
        s2.defaults
            .http
            .headers
            .insert("X-Run".into(), "{{run.id}}".into());
        if let Action::KafkaConsume(c) = &mut s2.scenarios[0].steps[2].action {
            c.match_
                .jsonpath
                .insert("$.orderId".into(), "{{steps.create.order_id}}".into());
        }
        let found: Vec<String> = templates_of(&s2)
            .iter()
            .map(|(p, _)| crate::suite::lines::path_string(p))
            .collect();
        assert_eq!(found[0], "defaults.http.headers.X-Run");
        assert!(
            found.contains(
                &"scenarios[0].steps[2].kafka_consume.match.jsonpath.$.orderId".to_string()
            )
        );
        assert!(
            !found.iter().any(|p| p.contains("extract")),
            "extract — JSONPath, не шаблон"
        );
    }
}
