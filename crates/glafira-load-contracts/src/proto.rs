//! Protobuf 3 → операции `grpc` (спека L1 §5.2, решение плана 2: свой разбор подмножества,
//! потому что крейт не читает диск). Неподдержанные конструкции дают предупреждение со строкой.

use std::collections::{BTreeMap, BTreeSet};

use glafira_load_model::registry::{
    Address, OpKind, OperationDraft, Source, SourceKind, Streaming,
};
use glafira_load_model::schema::{MAX_DEPTH, SchemaSummary};

use crate::{ContractInput, ParseError, ParseOptions, ParseResult, dir_of, join_path};

#[derive(Debug, Clone, PartialEq)]
enum Tok {
    Ident(String),
    Str(String),
    Num(String),
    Punct(char),
}

struct Token {
    tok: Tok,
    line: u32,
}

fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let b = src.as_bytes();
    let (mut i, mut line, mut out) = (0usize, 1u32, Vec::new());
    while i < b.len() {
        let c = b[i] as char;
        if c == '\n' {
            line += 1;
            i += 1;
        } else if c.is_whitespace() {
            i += 1;
        } else if src[i..].starts_with("//") {
            while i < b.len() && b[i] != b'\n' {
                i += 1;
            }
        } else if src[i..].starts_with("/*") {
            let end = src[i + 2..]
                .find("*/")
                .map(|e| i + 2 + e + 2)
                .unwrap_or(b.len());
            line += src[i..end].matches('\n').count() as u32;
            i = end;
        } else if c.is_ascii_alphabetic() || c == '_' || c == '.' {
            let start = i;
            while i < b.len()
                && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_' || b[i] == b'.')
            {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Ident(src[start..i].into()),
                line,
            });
        } else if c.is_ascii_digit() || c == '-' {
            let start = i;
            i += 1;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'.') {
                i += 1;
            }
            out.push(Token {
                tok: Tok::Num(src[start..i].into()),
                line,
            });
        } else if c == '"' || c == '\'' {
            let start = i + 1;
            i += 1;
            while i < b.len() && b[i] as char != c {
                if b[i] == b'\\' {
                    i += 1;
                }
                i += 1;
            }
            out.push(Token {
                tok: Tok::Str(src[start..i.min(b.len())].into()),
                line,
            });
            i += 1;
        } else if "{}()<>[]=;,:".contains(c) {
            out.push(Token {
                tok: Tok::Punct(c),
                line,
            });
            i += 1;
        } else {
            return Err(ParseError {
                message: format!("unexpected character {c:?}"),
                line: Some(line),
            });
        }
    }
    Ok(out)
}

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
                Tok::Ident(w) if w == "extend" => {
                    self.file
                        .warnings
                        .push((self.line(), "extend is not supported".into()));
                    self.next();
                    self.ident()?;
                    self.skip_block();
                }
                Tok::Punct(';') => {
                    self.next();
                }
                other => return Err(self.err(format!("unexpected {other:?} at top level"))),
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
                    while !self.is_punct('}') {
                        if self.is_ident("option") {
                            self.skip_statement();
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
                Tok::Ident(w) if w == "group" => {
                    self.file
                        .warnings
                        .push((self.line(), "group is not supported".into()));
                    self.skip_statement();
                }
                Tok::Ident(w) if w == "repeated" || w == "optional" || w == "required" => {
                    self.next();
                    msg.fields.push(self.field(w == "repeated")?);
                }
                Tok::Ident(_) => msg.fields.push(self.field(false)?),
                Tok::Punct(';') => {
                    self.next();
                }
                other => return Err(self.err(format!("unexpected {other:?} in message {name}"))),
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
        while !self.is_punct('}') {
            if self.is_ident("option") || self.is_ident("reserved") {
                self.skip_statement();
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
        while !self.is_punct('}') {
            if self.is_ident("option") {
                self.skip_statement();
                continue;
            }
            if !self.is_ident("rpc") {
                return Err(self.err("rpc expected in service"));
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
    let mut p = Parser {
        toks: lex(text)?,
        i: 0,
        file: File::default(),
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
        | "google.protobuf.BytesValue" => "string",
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

fn type_summary(
    name: &str,
    type_name: &str,
    scope: &str,
    file: &File,
    depth: usize,
    stack: &mut Vec<String>,
    unresolved: &mut BTreeSet<String>,
) -> SchemaSummary {
    if let Some(k) = scalar(type_name).or_else(|| well_known(type_name)) {
        return SchemaSummary::leaf(name, k);
    }
    let Some(fq) = resolve_type(type_name, scope, file) else {
        unresolved.insert(type_name.to_string());
        return SchemaSummary::leaf(name, format!("unresolved:{type_name}"));
    };
    if let Some(values) = file.enums.get(&fq) {
        let mut s = SchemaSummary::leaf(name, "enum");
        s.enum_values = values.clone();
        return s;
    }
    if stack.contains(&fq) {
        return SchemaSummary::leaf(
            name,
            format!("ref:{}", fq.rsplit('.').next().unwrap_or(&fq)),
        );
    }
    message_summary(name, &fq, file, depth, stack, unresolved)
}

fn message_summary(
    name: &str,
    fq: &str,
    file: &File,
    depth: usize,
    stack: &mut Vec<String>,
    unresolved: &mut BTreeSet<String>,
) -> SchemaSummary {
    let mut out = SchemaSummary::object(name, vec![]);
    if depth >= MAX_DEPTH {
        out.truncated = true;
        return out;
    }
    let Some(msg) = file.messages.get(fq) else {
        return out;
    };
    stack.push(fq.to_string());
    for f in &msg.fields {
        let child = match &f.map_value {
            Some(v) => {
                let mut m = SchemaSummary::leaf(&f.name, "map");
                m.children = vec![type_summary(
                    "value",
                    v,
                    fq,
                    file,
                    depth + 1,
                    stack,
                    unresolved,
                )];
                m
            }
            None => type_summary(
                &f.name,
                &f.type_name,
                fq,
                file,
                depth + 1,
                stack,
                unresolved,
            ),
        };
        out.children.push(if f.repeated {
            SchemaSummary::array(&f.name, child)
        } else {
            child
        });
    }
    stack.pop();
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
    let mut unresolved = BTreeSet::new();
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
            let mut stack = Vec::new();
            let mut req = type_summary(
                "request",
                &rpc.request,
                &file.package,
                &file,
                1,
                &mut stack,
                &mut unresolved,
            );
            let mut resp = type_summary(
                "response",
                &rpc.response,
                &file.package,
                &file,
                1,
                &mut stack,
                &mut unresolved,
            );
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
    for t in unresolved {
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
}
