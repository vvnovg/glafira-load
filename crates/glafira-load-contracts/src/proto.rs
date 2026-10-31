//! Protobuf 3 → операции `grpc` (спека L1 §5.2, решение плана 2: свой разбор подмножества,
//! потому что крейт не читает диск). Неподдержанные конструкции дают предупреждение со строкой.

use std::collections::{BTreeMap, BTreeSet};

use glafira_load_model::registry::{
    Address, OpKind, OperationDraft, Source, SourceKind, Streaming,
};
use glafira_load_model::schema::{MAX_CHILDREN, MAX_DEPTH, SchemaSummary};

mod lexer;

use lexer::{Tok, Token, lex};

use crate::{ContractInput, ParseError, ParseOptions, ParseResult, dir_of, join_path};

#[derive(Debug, Clone, Default)]
struct Field {
    name: String,
    type_name: String,
    repeated: bool,
    /// Тип значения у `map<K, V>`.
    map_value: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct Message {
    fields: Vec<Field>,
}

#[derive(Debug, Clone)]
struct Rpc {
    name: String,
    request: String,
    response: String,
    client_stream: bool,
    server_stream: bool,
    line: u32,
}

#[derive(Debug, Clone)]
struct Service {
    name: String,
    rpcs: Vec<Rpc>,
}

/// Разобранный файл; после слияния импортов — все известные сообщения и enum по полным именам.
#[derive(Debug, Default)]
struct File {
    package: String,
    imports: Vec<String>,
    messages: BTreeMap<String, Message>,
    enums: BTreeMap<String, Vec<String>>,
    services: Vec<Service>,
    warnings: Vec<(u32, String)>,
}

/// Ключевые слова, которые в теле `message` не поля и не из поддерживаемого подмножества.
const UNSUPPORTED_IN_MESSAGE: &[&str] = &[
    "extend", "group", "service", "rpc", "syntax", "package", "import", "edition",
];

struct Parser {
    toks: Vec<Token>,
    i: usize,
    file: File,
}

impl Parser {
    fn peek(&self) -> Option<&Tok> {
        self.toks.get(self.i).map(|t| &t.tok)
    }
    fn line(&self) -> u32 {
        self.toks
            .get(self.i)
            .or(self.toks.last())
            .map(|t| t.line)
            .unwrap_or(1)
    }
    fn next(&mut self) -> Option<Tok> {
        let t = self.toks.get(self.i).map(|t| t.tok.clone());
        self.i += 1;
        t
    }
    fn err(&self, message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
            line: Some(self.line()),
        }
    }
    fn ident(&mut self) -> Result<String, ParseError> {
        match self.next() {
            Some(Tok::Ident(s)) => Ok(s),
            other => Err(self.err(format!("identifier expected, got {other:?}"))),
        }
    }
    fn punct(&mut self, c: char) -> Result<(), ParseError> {
        match self.next() {
            Some(Tok::Punct(p)) if p == c => Ok(()),
            other => Err(self.err(format!("{c:?} expected, got {other:?}"))),
        }
    }
    fn is_punct(&self, c: char) -> bool {
        self.peek() == Some(&Tok::Punct(c))
    }
    fn is_ident(&self, s: &str) -> bool {
        matches!(self.peek(), Some(Tok::Ident(x)) if x == s)
    }
    /// До `;` на нулевой глубине скобок (опции с `{ … }` внутри).
    fn skip_statement(&mut self) {
        let mut depth = 0i32;
        while let Some(t) = self.next() {
            match t {
                Tok::Punct('{') | Tok::Punct('(') | Tok::Punct('[') => depth += 1,
                Tok::Punct('}') | Tok::Punct(')') | Tok::Punct(']') => depth -= 1,
                Tok::Punct(';') if depth <= 0 => return,
                _ => {}
            }
        }
    }
    /// Блок `{ … }` целиком (открывающая скобка ещё не съедена).
    fn skip_block(&mut self) {
        let mut depth = 0i32;
        while let Some(t) = self.next() {
            match t {
                Tok::Punct('{') => depth += 1,
                Tok::Punct('}') => {
                    depth -= 1;
                    if depth <= 0 {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
    /// Неизвестная конструкция: первый токен съедается всегда (прогресс), дальше — до `;` на
    /// нулевой глубине или до конца первого `{ … }`. Закрывающая `}` объемлющего блока не
    /// трогается. Пустая `;` после блока (`rpc X(A) returns (B) {};`) потом пропускается молча.
    fn skip_construct(&mut self) {
        let mut depth = 0i32;
        let mut first = true;
        loop {
            if !first && depth == 0 && self.is_punct('}') {
                return;
            }
            let Some(t) = self.next() else { return };
            first = false;
            match t {
                Tok::Punct('{') | Tok::Punct('(') | Tok::Punct('[') => depth += 1,
                Tok::Punct('}') => {
                    depth -= 1;
                    if depth <= 0 {
                        return;
                    }
                }
                Tok::Punct(')') | Tok::Punct(']') => depth -= 1,
                Tok::Punct(';') if depth <= 0 => return,
                _ => {}
            }
        }
    }
    /// Предупреждение `unsupported construct` для текущего токена и пропуск конструкции.
    fn unsupported(&mut self) {
        let line = self.line();
        let what = match self.peek() {
            Some(Tok::Ident(s)) | Some(Tok::Num(s)) => s.clone(),
            Some(Tok::Str(s)) => format!("\"{s}\""),
            Some(Tok::Punct(c)) => c.to_string(),
            None => String::new(),
        };
        self.file
            .warnings
            .push((line, format!("unsupported construct '{what}'")));
        self.skip_construct();
    }
    /// `true` на закрывающей `}` блока; конец файла внутри блока — ошибка.
    fn block_end(&self, what: &str) -> Result<bool, ParseError> {
        if self.peek().is_none() {
            return Err(self.err(format!("unexpected end of file in {what}")));
        }
        Ok(self.is_punct('}'))
    }
    /// Следующие три токена — `Ident Ident =` (поле `type name = N`).
    fn looks_like_field(&self) -> bool {
        matches!(
            (
                self.toks.get(self.i).map(|t| &t.tok),
                self.toks.get(self.i + 1).map(|t| &t.tok),
                self.toks.get(self.i + 2).map(|t| &t.tok)
            ),
            (
                Some(Tok::Ident(_)),
                Some(Tok::Ident(_)),
                Some(Tok::Punct('='))
            )
        )
    }
    fn skip_field_options(&mut self) {
        if self.is_punct('[') {
            let mut depth = 0i32;
            while let Some(t) = self.next() {
                match t {
                    Tok::Punct('[') => depth += 1,
                    Tok::Punct(']') => {
                        depth -= 1;
                        if depth <= 0 {
                            return;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn file(&mut self) -> Result<(), ParseError> {
        while let Some(t) = self.peek().cloned() {
            match t {
                Tok::Ident(w) if w == "syntax" => {
                    self.next();
                    self.punct('=')?;
                    if let Some(Tok::Str(s)) = self.next()
                        && s != "proto3"
                    {
                        self.file.warnings.push((
                            self.line(),
                            format!("syntax {s}: only proto3 is supported, parsing anyway"),
                        ));
                    }
                    self.punct(';')?;
                }
                Tok::Ident(w) if w == "package" => {
                    self.next();
                    self.file.package = self.ident()?;
                    self.punct(';')?;
                }
                Tok::Ident(w) if w == "import" => {
                    self.next();
                    if self.is_ident("public") || self.is_ident("weak") {
                        self.next();
                    }
                    match self.next() {
                        Some(Tok::Str(p)) => self.file.imports.push(p),
                        other => {
                            return Err(self.err(format!("import path expected, got {other:?}")));
                        }
                    }
                    self.punct(';')?;
                }
                Tok::Ident(w) if w == "option" => self.skip_statement(),
                Tok::Ident(w) if w == "message" => {
                    self.next();
                    let scope = self.file.package.clone();
                    self.message(&scope)?;
                }
                Tok::Ident(w) if w == "enum" => {
                    self.next();
                    let scope = self.file.package.clone();
                    self.enum_def(&scope)?;
                }
                Tok::Ident(w) if w == "service" => {
                    self.next();
                    self.service()?;
                }
                Tok::Punct(';') => {
                    self.next();
                }
                // Лишняя `}` на верхнем уровне: предупреждение и один токен.
                Tok::Punct('}') => {
                    self.file
                        .warnings
                        .push((self.line(), "unsupported construct '}'".into()));
                    self.next();
                }
                // `edition`, `extend` и прочее вне подмножества.
                _ => self.unsupported(),
            }
        }
        Ok(())
    }

    fn fq(scope: &str, name: &str) -> String {
        if scope.is_empty() {
            name.into()
        } else {
            format!("{scope}.{name}")
        }
    }

    fn message(&mut self, scope: &str) -> Result<(), ParseError> {
        let name = self.ident()?;
        let fq = Self::fq(scope, &name);
        self.punct('{')?;
        let mut msg = Message::default();
        while !self.is_punct('}') {
            let Some(t) = self.peek().cloned() else {
                return Err(self.err("unexpected end of file in message"));
            };
            match t {
                Tok::Ident(w) if w == "message" => {
                    self.next();
                    self.message(&fq)?;
                }
                Tok::Ident(w) if w == "enum" => {
                    self.next();
                    self.enum_def(&fq)?;
                }
                Tok::Ident(w) if w == "oneof" => {
                    self.next();
                    self.ident()?;
                    self.punct('{')?;
                    while !self.block_end("oneof")? {
                        if self.is_ident("option") {
                            self.skip_statement();
                        } else if self.is_punct(';') {
                            self.next();
                        } else if self.is_ident("group") || !self.looks_like_field() {
                            self.unsupported();
                        } else {
                            msg.fields.push(self.field(false)?);
                        }
                    }
                    self.punct('}')?;
                }
                Tok::Ident(w) if w == "reserved" || w == "extensions" || w == "option" => {
                    self.skip_statement()
                }
                Tok::Ident(w) if w == "map" => {
                    self.next();
                    self.punct('<')?;
                    self.ident()?;
                    self.punct(',')?;
                    let value = self.ident()?;
                    self.punct('>')?;
                    let fname = self.ident()?;
                    self.punct('=')?;
                    self.next();
                    self.skip_field_options();
                    self.punct(';')?;
                    msg.fields.push(Field {
                        name: fname,
                        type_name: "map".into(),
                        repeated: false,
                        map_value: Some(value),
                    });
                }
                // Конструкции вне подмножества (proto2 `group`, `extend`, чужие операторы).
                Tok::Ident(w) if UNSUPPORTED_IN_MESSAGE.contains(&w.as_str()) => self.unsupported(),
                Tok::Ident(w) if w == "repeated" || w == "optional" || w == "required" => {
                    self.next();
                    if self.is_ident("group") {
                        self.unsupported();
                    } else {
                        msg.fields.push(self.field(w == "repeated")?);
                    }
                }
                Tok::Ident(_) => msg.fields.push(self.field(false)?),
                Tok::Punct(';') => {
                    self.next();
                }
                // Строка, число, лишняя пунктуация: предупреждение и восстановление.
                _ => self.unsupported(),
            }
        }
        self.punct('}')?;
        self.file.messages.insert(fq, msg);
        Ok(())
    }

    fn field(&mut self, repeated: bool) -> Result<Field, ParseError> {
        let type_name = self.ident()?;
        let name = self.ident()?;
        self.punct('=')?;
        self.next();
        self.skip_field_options();
        self.punct(';')?;
        Ok(Field {
            name,
            type_name,
            repeated,
            map_value: None,
        })
    }

    fn enum_def(&mut self, scope: &str) -> Result<(), ParseError> {
        let name = self.ident()?;
        self.punct('{')?;
        let mut values = Vec::new();
        while !self.block_end("enum")? {
            if self.is_ident("option") || self.is_ident("reserved") {
                self.skip_statement();
                continue;
            }
            if self.is_punct(';') {
                self.next();
                continue;
            }
            if !matches!(
                (self.peek(), self.toks.get(self.i + 1).map(|t| &t.tok)),
                (Some(Tok::Ident(_)), Some(Tok::Punct('=')))
            ) {
                self.unsupported();
                continue;
            }
            let v = self.ident()?;
            self.punct('=')?;
            self.next();
            self.skip_field_options();
            self.punct(';')?;
            values.push(v);
        }
        self.punct('}')?;
        self.file.enums.insert(Self::fq(scope, &name), values);
        Ok(())
    }

    fn service(&mut self) -> Result<(), ParseError> {
        let name = self.ident()?;
        self.punct('{')?;
        let mut rpcs = Vec::new();
        while !self.block_end("service")? {
            if self.is_ident("option") {
                self.skip_statement();
                continue;
            }
            if self.is_punct(';') {
                self.next();
                continue;
            }
            if !self.is_ident("rpc") {
                self.unsupported();
                continue;
            }
            let line = self.line();
            self.next();
            let rname = self.ident()?;
            self.punct('(')?;
            let client_stream = self.is_ident("stream") && self.next().is_some();
            let request = self.ident()?;
            self.punct(')')?;
            if !self.is_ident("returns") {
                return Err(self.err("returns expected"));
            }
            self.next();
            self.punct('(')?;
            let server_stream = self.is_ident("stream") && self.next().is_some();
            let response = self.ident()?;
            self.punct(')')?;
            if self.is_punct('{') {
                self.skip_block();
            } else {
                self.punct(';')?;
            }
            rpcs.push(Rpc {
                name: rname,
                request,
                response,
                client_stream,
                server_stream,
                line,
            });
        }
        self.punct('}')?;
        self.file.services.push(Service { name, rpcs });
        Ok(())
    }
}

fn parse_text(text: &str) -> Result<File, ParseError> {
    let (toks, lex_warnings) = lex(text);
    let mut p = Parser {
        toks,
        i: 0,
        file: File {
            warnings: lex_warnings,
            ..File::default()
        },
    };
    p.file()?;
    Ok(p.file)
}

/// Well-known types: их `import` не ищется, тип сворачивается в скаляр JSON-отображения.
fn well_known(name: &str) -> Option<&'static str> {
    Some(match name.trim_start_matches('.') {
        "google.protobuf.Timestamp"
        | "google.protobuf.Duration"
        | "google.protobuf.StringValue"
        | "google.protobuf.BytesValue"
        | "google.protobuf.FieldMask" => "string",
        "google.protobuf.Empty"
        | "google.protobuf.Any"
        | "google.protobuf.Struct"
        | "google.protobuf.Value" => "object",
        "google.protobuf.Int32Value"
        | "google.protobuf.Int64Value"
        | "google.protobuf.UInt32Value"
        | "google.protobuf.UInt64Value" => "integer",
        "google.protobuf.FloatValue" | "google.protobuf.DoubleValue" => "number",
        "google.protobuf.BoolValue" => "boolean",
        _ => return None,
    })
}

fn scalar(name: &str) -> Option<&'static str> {
    Some(match name {
        "double" | "float" => "number",
        "int32" | "int64" | "uint32" | "uint64" | "sint32" | "sint64" | "fixed32" | "fixed64"
        | "sfixed32" | "sfixed64" => "integer",
        "bool" => "boolean",
        "string" => "string",
        "bytes" => "bytes",
        _ => return None,
    })
}

/// Имя типа из области `scope` (`pkg.Outer`): проверяются `scope.T`, родители, затем `T` как есть.
fn resolve_type(name: &str, scope: &str, file: &File) -> Option<String> {
    if let Some(n) = name.strip_prefix('.') {
        return (file.messages.contains_key(n) || file.enums.contains_key(n))
            .then(|| n.to_string());
    }
    let mut scope = scope.to_string();
    loop {
        let candidate = if scope.is_empty() {
            name.to_string()
        } else {
            format!("{scope}.{name}")
        };
        if file.messages.contains_key(&candidate) || file.enums.contains_key(&candidate) {
            return Some(candidate);
        }
        if scope.is_empty() {
            return None;
        }
        scope = scope
            .rsplit_once('.')
            .map(|(p, _)| p.to_string())
            .unwrap_or_default();
    }
}

/// Верхняя граница числа полей-узлов одной сводки: ширина на уровне (`MAX_CHILDREN`) и глубина
/// (`MAX_DEPTH`) по отдельности не ограничивают сообщения, вложенные друг в друга ширококорневыми
/// полями (`F^5` узлов). Когда бюджет исчерпан, сообщения-поля сворачиваются в пустой
/// `object` с `truncated`.
const MAX_NODES: usize = 10_000;

/// Состояние построения одной сводки схемы.
struct Ctx<'a> {
    file: &'a File,
    stack: Vec<String>,
    unresolved: BTreeSet<String>,
    nodes: usize,
}

fn type_summary(
    ctx: &mut Ctx,
    name: &str,
    type_name: &str,
    scope: &str,
    depth: usize,
) -> SchemaSummary {
    if type_name.trim_start_matches('.') == "google.protobuf.ListValue" {
        return SchemaSummary::array(name, SchemaSummary::leaf("items", "any"));
    }
    if let Some(k) = scalar(type_name).or_else(|| well_known(type_name)) {
        return SchemaSummary::leaf(name, k);
    }
    let Some(fq) = resolve_type(type_name, scope, ctx.file) else {
        ctx.unresolved.insert(type_name.to_string());
        return SchemaSummary::leaf(name, format!("unresolved:{type_name}"));
    };
    if let Some(values) = ctx.file.enums.get(&fq) {
        let mut s = SchemaSummary::leaf(name, "enum");
        s.enum_values = values.clone();
        return s;
    }
    if ctx.stack.contains(&fq) {
        return SchemaSummary::leaf(
            name,
            format!("ref:{}", fq.rsplit('.').next().unwrap_or(&fq)),
        );
    }
    if ctx.nodes >= MAX_NODES {
        let mut s = SchemaSummary::object(name, vec![]);
        s.truncated = true;
        return s;
    }
    message_summary(ctx, name, &fq, depth)
}

fn message_summary(ctx: &mut Ctx, name: &str, fq: &str, depth: usize) -> SchemaSummary {
    let mut out = SchemaSummary::object(name, vec![]);
    if depth >= MAX_DEPTH {
        out.truncated = true;
        return out;
    }
    let Some(msg) = ctx.file.messages.get(fq) else {
        return out;
    };
    ctx.stack.push(fq.to_string());
    for f in &msg.fields {
        if out.children.len() >= MAX_CHILDREN {
            out.truncated = true;
            break;
        }
        ctx.nodes += 1;
        let child = match &f.map_value {
            Some(v) => {
                let mut m = SchemaSummary::leaf(&f.name, "map");
                m.children = vec![type_summary(ctx, "value", v, fq, depth + 1)];
                m
            }
            None => type_summary(ctx, &f.name, &f.type_name, fq, depth + 1),
        };
        out.children.push(if f.repeated {
            SchemaSummary::array(&f.name, child)
        } else {
            child
        });
    }
    ctx.stack.pop();
    out
}

/// Разбирает файл и его импорты (кроме `google/protobuf/*`), сливая сообщения и enum.
fn load(
    path: &str,
    bytes: &[u8],
    input: &ContractInput,
    opts: &ParseOptions,
    visited: &mut BTreeSet<String>,
    out: &mut ParseResult,
) -> Result<File, ParseError> {
    let text = std::str::from_utf8(bytes).map_err(|e| ParseError {
        message: format!("not UTF-8: {e}"),
        line: None,
    })?;
    let mut file = parse_text(text)?;
    for (line, w) in std::mem::take(&mut file.warnings) {
        out.warn(path, Some(line), w);
    }
    for import in file.imports.clone() {
        if import.starts_with("google/protobuf/") || !visited.insert(import.clone()) {
            continue;
        }
        let candidates: Vec<String> = opts
            .include
            .iter()
            .map(|inc| join_path(inc, &import))
            .chain([join_path(dir_of(path), &import), import.clone()])
            .collect();
        let Some((found, bytes)) = candidates
            .iter()
            .find_map(|c| (input.read)(c).map(|b| (c.clone(), b)))
        else {
            out.warn(path, None, format!("import not found: {import}"));
            continue;
        };
        match load(&found, &bytes, input, opts, visited, out) {
            Ok(imported) => {
                for (k, v) in imported.messages {
                    file.messages.entry(k).or_insert(v);
                }
                for (k, v) in imported.enums {
                    file.enums.entry(k).or_insert(v);
                }
            }
            Err(e) => out.warn(&found, e.line, e.message),
        }
    }
    Ok(file)
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let mut out = ParseResult::default();
    let mut visited = BTreeSet::from([input.path.to_string()]);
    let file = load(input.path, input.bytes, input, opts, &mut visited, &mut out)?;
    let mut ctx = Ctx {
        file: &file,
        stack: Vec::new(),
        unresolved: BTreeSet::new(),
        nodes: 0,
    };
    for svc in &file.services {
        let service_fq = Parser::fq(&file.package, &svc.name);
        for rpc in &svc.rpcs {
            let mut draft = OperationDraft::new(
                &opts.service,
                OpKind::Grpc,
                Address::Grpc {
                    service: service_fq.clone(),
                    method: rpc.name.clone(),
                },
                Source {
                    kind: SourceKind::Proto,
                    path: input.path.into(),
                    line: Some(rpc.line),
                },
            );
            draft.name = rpc.name.clone();
            ctx.stack.clear();
            ctx.nodes = 0;
            let mut req = type_summary(&mut ctx, "request", &rpc.request, &file.package, 1);
            ctx.nodes = 0;
            let mut resp = type_summary(&mut ctx, "response", &rpc.response, &file.package, 1);
            req.limit();
            resp.limit();
            draft.request = Some(req);
            draft.response = Some(resp);
            draft.streaming = match (rpc.client_stream, rpc.server_stream) {
                (true, true) => Some(Streaming::Bidi),
                (true, false) => Some(Streaming::Client),
                (false, true) => Some(Streaming::Server),
                (false, false) => None,
            };
            out.operations.push(draft);
        }
    }
    for t in ctx.unresolved {
        out.warn(input.path, None, format!("type not found: {t}"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ORDERS: &str =
        include_str!("../../../dev/fixtures/orders-proto/proto/orders/v1/orders.proto");
    const MONEY: &str = include_str!("../../../dev/fixtures/orders-proto/proto/common/money.proto");

    fn opts() -> ParseOptions {
        ParseOptions {
            service: "orders".into(),
            include: vec!["proto".into()],
            ..Default::default()
        }
    }

    fn child<'a>(s: &'a SchemaSummary, name: &str) -> &'a SchemaSummary {
        s.children
            .iter()
            .find(|c| c.name == name)
            .unwrap_or_else(|| {
                panic!(
                    "no field {name} in {:?}",
                    s.children.iter().map(|c| &c.name).collect::<Vec<_>>()
                )
            })
    }

    #[test]
    fn services_messages_imports_and_streaming() {
        let read = |p: &str| (p == "proto/common/money.proto").then(|| MONEY.as_bytes().to_vec());
        let input = ContractInput {
            path: "proto/orders/v1/orders.proto",
            bytes: ORDERS.as_bytes(),
            read: &read,
        };
        let out = parse(&input, &opts()).unwrap();
        assert_eq!(out.warnings, vec![], "{:?}", out.warnings);
        assert_eq!(
            out.operations
                .iter()
                .map(|o| o.id.as_str())
                .collect::<Vec<_>>(),
            [
                "grpc:orders:orders.v1.OrderService/Create",
                "grpc:orders:orders.v1.OrderService/Get",
                "grpc:orders:orders.v1.OrderService/Watch",
            ]
        );
        let create = &out.operations[0];
        assert_eq!(create.source.line, Some(39));
        let order = child(create.request.as_ref().unwrap(), "order");
        assert_eq!(order.kind, "object");
        assert_eq!(child(order, "id").kind, "string");
        let items = child(order, "items");
        assert_eq!(items.kind, "array");
        let price = child(&items.children[0], "price");
        assert_eq!(
            price
                .children
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["currency", "units", "nanos"],
            "Money из импорта"
        );
        assert_eq!(child(order, "status").enum_values, ["NEW", "PAID"]);
        assert_eq!(
            (
                child(order, "labels").kind.as_str(),
                child(order, "labels").children[0].kind.as_str()
            ),
            ("map", "string")
        );
        assert_eq!(
            child(order, "created_at").kind,
            "string",
            "well-known Timestamp"
        );
        assert!(
            order.children.iter().any(|c| c.name == "customer_id")
                && order.children.iter().any(|c| c.name == "company_id"),
            "поля oneof лежат плоско"
        );
        assert_eq!(out.operations[2].streaming, Some(Streaming::Server));
        assert_eq!(out.operations[1].streaming, None);
    }

    #[test]
    fn missing_import_warns_and_marks_unresolved() {
        let read = |_: &str| None;
        let input = ContractInput {
            path: "proto/orders/v1/orders.proto",
            bytes: ORDERS.as_bytes(),
            read: &read,
        };
        let out = parse(&input, &opts()).unwrap();
        let price = child(
            &child(
                child(out.operations[0].request.as_ref().unwrap(), "order"),
                "items",
            )
            .children[0],
            "price",
        );
        assert_eq!(price.kind, "unresolved:common.Money");
        let msgs: Vec<&str> = out.warnings.iter().map(|w| w.message.as_str()).collect();
        assert!(
            msgs.contains(&"import not found: common/money.proto"),
            "{msgs:?}"
        );
        assert!(msgs.contains(&"type not found: common.Money"), "{msgs:?}");
    }

    #[test]
    fn syntax_errors_carry_a_line() {
        let read = |_: &str| None;
        let e = parse(
            &ContractInput {
                path: "a.proto",
                bytes: b"syntax = \"proto3\";\nmessage A {\n  string = 1;\n}\n",
                read: &read,
            },
            &opts(),
        )
        .unwrap_err();
        assert_eq!(e.line, Some(3));
        let out = parse(&ContractInput { path: "a.proto", bytes: b"syntax = \"proto2\";\nmessage A { optional string x = 1; }\nservice S { rpc Do(A) returns (A); }", read: &read }, &opts()).unwrap();
        assert!(out.warnings[0].message.contains("proto2"));
        assert_eq!(out.operations.len(), 1);
    }

    #[test]
    fn recursive_messages_stop_at_a_reference() {
        let read = |_: &str| None;
        let src = b"syntax = \"proto3\"; package t; message Node { Node next = 1; } service S { rpc Walk(Node) returns (Node); }";
        let out = parse(
            &ContractInput {
                path: "a.proto",
                bytes: src,
                read: &read,
            },
            &opts(),
        )
        .unwrap();
        assert_eq!(
            child(out.operations[0].request.as_ref().unwrap(), "next").kind,
            "ref:Node"
        );
    }

    fn run(src: &str) -> ParseResult {
        let read = |_: &str| None;
        parse(
            &ContractInput {
                path: "a.proto",
                bytes: src.as_bytes(),
                read: &read,
            },
            &opts(),
        )
        .unwrap()
    }

    fn count_nodes(s: &SchemaSummary) -> usize {
        1 + s.children.iter().map(count_nodes).sum::<usize>()
    }

    fn max_width(s: &SchemaSummary) -> usize {
        s.children
            .iter()
            .map(max_width)
            .max()
            .unwrap_or(0)
            .max(s.children.len())
    }

    #[test]
    fn bom_unknown_constructs_and_stray_semicolons_are_warnings() {
        let src = "\u{feff}syntax = \"proto3\";\nedition = \"2023\";\npackage t;\n\
                   enum E {\n  A = 0;;\n  B = 1;\n}\n\
                   service S {\n  rpc Do(M) returns (M) {};\n  ;\n  frobnicate X;\n  rpc Two(M) returns (M);\n}\n\
                   message M { string x = 1; }\n";
        let out = run(src);
        let ops: Vec<&str> = out.operations.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(
            ops,
            ["grpc:orders:t.S/Do", "grpc:orders:t.S/Two"],
            "{:?}",
            out.warnings
        );
        assert_eq!(out.operations[0].source.line, Some(9));
        let edition = out
            .warnings
            .iter()
            .find(|w| w.message.contains("edition"))
            .unwrap_or_else(|| panic!("{:?}", out.warnings));
        assert_eq!(edition.line, Some(2));
        let frob = out
            .warnings
            .iter()
            .find(|w| w.message.contains("frobnicate"))
            .unwrap();
        assert_eq!(frob.line, Some(11));
        assert_eq!(out.warnings.len(), 2, "пустые `;` — без предупреждений");
    }

    #[test]
    fn exponents_and_exotic_punctuation_lex_fine() {
        let src = "syntax = \"proto3\";\npackage t;\n\
                   option (a.b) = 1e+10;\noption (a.c) = 2E-3;\n\
                   option (x) = { url: [type.googleapis.com/foo.Bar] mask: a|b at: @c };\n\
                   message M {\n  string id = 1 [(google.api.http) = {get: \"/v1/x/{id}\" body: '*'}, (v) = 1e+5];\n  int32 n = 2;\n}\n\
                   service S { rpc Do(M) returns (M) { option (google.api.http) = { get: \"/v1/x/{id}\" }; } }\n";
        let out = run(src);
        assert_eq!(out.warnings, vec![], "{:?}", out.warnings);
        let req = out.operations[0].request.as_ref().unwrap();
        assert_eq!(
            req.children
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["id", "n"]
        );
    }

    #[test]
    fn unsupported_construct_in_message_warns_and_parsing_continues() {
        let src = "syntax = \"proto3\";\npackage t;\nmessage M {\n  string a = 1;\n  extend Foo {\n    int32 z = 100;\n  }\n  optional group G = 2 {\n    string q = 3;\n  }\n  string b = 4;\n}\nservice S { rpc Do(M) returns (M); }\n";
        let out = run(src);
        let req = out.operations[0].request.as_ref().unwrap();
        assert_eq!(
            req.children
                .iter()
                .map(|c| c.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        let lines: Vec<(Option<u32>, &str)> = out
            .warnings
            .iter()
            .map(|w| (w.line, w.message.as_str()))
            .collect();
        assert_eq!(
            lines,
            [
                (Some(5), "unsupported construct 'extend'"),
                (Some(8), "unsupported construct 'group'")
            ]
        );
    }

    #[test]
    fn strings_keep_line_numbers_and_unterminated_string_warns() {
        let src = "syntax = \"proto3\";\noption (x) = \"multi\nline \\\n cont\";\nmessage M { string a = 1; }\nservice S { rpc Do(M) returns (M); }\n";
        let out = run(src);
        assert_eq!(out.operations[0].source.line, Some(6), "{:?}", out.warnings);
        let out =
            run("syntax = \"proto3\";\nmessage M { string a = 1; }\noption (x) = \"oops;\nmore");
        assert!(
            out.warnings
                .iter()
                .any(|w| w.line == Some(3) && w.message.contains("unterminated")),
            "{:?}",
            out.warnings
        );
    }

    #[test]
    fn wide_message_is_truncated_at_max_children() {
        let mut src = String::from("syntax = \"proto3\"; message Big {\n");
        for i in 0..250 {
            src.push_str(&format!("  string f{i} = {};\n", i + 1));
        }
        src.push_str("}\nservice S { rpc Do(Big) returns (Big); }\n");
        let out = run(&src);
        let req = out.operations[0].request.as_ref().unwrap();
        assert_eq!(req.kind, "object");
        assert_eq!(req.children.len(), 200);
        assert!(req.truncated);
        let mut exact = String::from("syntax = \"proto3\"; message Big {\n");
        for i in 0..200 {
            exact.push_str(&format!("  string f{i} = {};\n", i + 1));
        }
        exact.push_str("}\nservice S { rpc Do(Big) returns (Big); }\n");
        let out = run(&exact);
        let req = out.operations[0].request.as_ref().unwrap();
        assert_eq!((req.children.len(), req.truncated), (200, false));
    }

    #[test]
    fn wide_nested_messages_do_not_blow_up() {
        let mut src = String::from("syntax = \"proto3\";\n");
        for level in 1..=6 {
            let name = if level == 1 {
                "W".to_string()
            } else {
                format!("W{level}")
            };
            src.push_str(&format!("message {name} {{\n"));
            for i in 0..30 {
                if level < 6 {
                    src.push_str(&format!("  W{} f{i} = {};\n", level + 1, i + 1));
                } else {
                    src.push_str(&format!("  string f{i} = {};\n", i + 1));
                }
            }
            src.push_str("}\n");
        }
        src.push_str("service S { rpc Do(W) returns (W); }\n");
        let started = std::time::Instant::now();
        let out = run(&src);
        assert!(started.elapsed().as_secs() < 5, "{:?}", started.elapsed());
        for s in [
            out.operations[0].request.as_ref().unwrap(),
            out.operations[0].response.as_ref().unwrap(),
        ] {
            assert!(max_width(s) <= 200);
            let n = count_nodes(s);
            assert!(n < 50_000, "{n} nodes");
            assert!(n > 100, "{n} nodes");
        }
    }

    #[test]
    fn client_and_bidi_streaming_and_extra_well_known_types() {
        let src = "syntax = \"proto3\"; package t;\nmessage M { google.protobuf.ListValue l = 1; google.protobuf.FieldMask m = 2; }\nservice S {\n  rpc Up(stream M) returns (M);\n  rpc Chat(stream M) returns (stream M);\n  rpc Down(M) returns (stream M);\n}\n";
        let out = run(src);
        assert_eq!(
            out.operations
                .iter()
                .map(|o| o.streaming)
                .collect::<Vec<_>>(),
            [
                Some(Streaming::Client),
                Some(Streaming::Bidi),
                Some(Streaming::Server)
            ]
        );
        let req = out.operations[0].request.as_ref().unwrap();
        let l = child(req, "l");
        assert_eq!(
            (l.kind.as_str(), l.children[0].kind.as_str()),
            ("array", "any")
        );
        assert_eq!(child(req, "m").kind, "string");
    }
}
