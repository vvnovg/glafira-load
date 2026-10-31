//! SQL-миграции → справочник таблиц (спека L1 §5.2, решение плана 10 про Liquibase).
//! Операций `sql` не создаёт: их задаёт человек, а таблицы нужны как подсказки.

use sqlparser::ast::{AlterTableOperation, ColumnOption, ObjectType, Statement};
use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

use crate::{
    Column, ContractInput, ParseError, ParseOptions, ParseResult, Table, dir_of, join_path,
};

/// Символ, который может входить в идентификатор (для распознавания `$tag$`).
fn is_ident_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// Если с позиции `i` начинается разделитель долларовых кавычек (`$$` или `$tag$`),
/// возвращает его целиком. Тег — `[A-Za-z_][A-Za-z0-9_]*`; `$1` и `a$b$` открывающими разделителями не считаются.
///
/// `open` — уже открытый разделитель: тогда ищем только его закрытие, и сравнение идёт напрямую,
/// без проверки предыдущего символа (`END$$` и `x$tag$` закрывают строку).
fn dollar_delimiter(chars: &[char], i: usize, open: Option<&[char]>) -> Option<Vec<char>> {
    if chars.get(i) != Some(&'$') {
        return None;
    }
    if let Some(open) = open {
        return chars[i..].starts_with(open).then(|| open.to_vec());
    }
    if i > 0 && (is_ident_char(chars[i - 1]) || chars[i - 1] == '$') {
        return None;
    }
    let mut j = i + 1;
    if let Some(&c) = chars.get(j)
        && (c.is_ascii_alphabetic() || c == '_')
    {
        while chars.get(j).is_some_and(|&c| is_ident_char(c)) {
            j += 1;
        }
    }
    (chars.get(j) == Some(&'$')).then(|| chars[i..=j].to_vec())
}

/// Разбивает текст на выражения по `;` вне кавычек (`'…'`, `"…"`, `` `…` ``),
/// `$$ … $$` / `$tag$ … $tag$` и комментариев (`-- …`, `/* … */`), с номером строки начала
/// выражения (первый значащий символ). Ведущий BOM отбрасывается.
///
/// Обратная косая: в `E'…'` она экранирует следующий символ; в обычных `'…'` — буквальный символ
/// (`standard_conforming_strings=on`, умолчание PostgreSQL), единственное экранирование — `''`.
/// Для файлов в стиле MySQL (`'it\'s'`): если при буквальном правиле кавычка к концу текста осталась
/// открытой, разбор повторяется с экранированием `\` во всех `'…'`, и берётся его результат.
fn split_statements(text: &str) -> Vec<(u32, String)> {
    let (parts, balanced) = split_with(text, false);
    if balanced {
        parts
    } else {
        split_with(text, true).0
    }
}

/// Один проход разбиения. `escape_all` — `\` экранирует и в обычных `'…'`.
/// Второе значение — закрыта ли `'…'`-строка к концу текста.
fn split_with(text: &str, escape_all: bool) -> (Vec<(u32, String)>, bool) {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = Vec::new();
    let (mut cur, mut start_line, mut line) = (String::new(), 1u32, 1u32);
    let (mut in_quote, mut in_ident) = (false, None::<char>);
    // Открытая строка — `E'…'`: в ней `\` экранирует следующий символ.
    let mut in_estr = false;
    // Открытый долларовый разделитель (`$$` или `$tag$`), которым надо закрыть строку.
    let mut in_dollar: Option<Vec<char>> = None;
    let (mut in_comment, mut in_block) = (false, false);
    // Встретился ли в текущем выражении значащий (не пробел и не комментарий) символ.
    let mut started = false;
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        // Символ — часть комментария: тогда он не влияет ни на кавычки, ни на `;`, ни на номер строки начала.
        let mut is_comment = in_comment || in_block;
        if c == '\n' {
            line += 1;
            in_comment = false;
        }
        let plain =
            !in_quote && in_ident.is_none() && in_dollar.is_none() && !in_comment && !in_block;
        if in_block && c == '*' && next == Some('/') {
            // Конец блочного комментария: закрывающую пару переносим в текст целиком.
            cur.push_str("*/");
            i += 2;
            in_block = false;
            continue;
        }
        if plain && c == '-' && next == Some('-') {
            in_comment = true;
            is_comment = true;
        } else if plain && c == '/' && next == Some('*') {
            in_block = true;
            is_comment = true;
        } else if !is_comment {
            if in_quote && c == '\\' && (escape_all || in_estr) {
                // Экранированный символ внутри строки: пара не меняет состояние кавычек.
                if let Some(n) = next {
                    if n == '\n' {
                        line += 1;
                    }
                    cur.push(c);
                    cur.push(n);
                    i += 2;
                    continue;
                }
            } else if c == '\'' && in_dollar.is_none() && in_ident.is_none() {
                if !in_quote {
                    // `E` не как хвост идентификатора и сразу перед кавычкой.
                    in_estr = i > 0
                        && matches!(chars[i - 1], 'e' | 'E')
                        && (i < 2 || !is_ident_char(chars[i - 2]));
                }
                in_quote = !in_quote;
            } else if (c == '"' || c == '`') && in_dollar.is_none() && !in_quote {
                match in_ident {
                    None => in_ident = Some(c),
                    Some(q) if q == c => in_ident = None,
                    Some(_) => {}
                }
            } else if c == '$' && !in_quote && in_ident.is_none() {
                // Открытие `$tag$` или закрытие тем же разделителем; чужой `$…$` внутри — просто текст.
                if let Some(d) = dollar_delimiter(&chars, i, in_dollar.as_deref()) {
                    if !started {
                        started = true;
                        start_line = line;
                    }
                    in_dollar = if in_dollar.is_some() {
                        None
                    } else {
                        Some(d.clone())
                    };
                    cur.extend(d.iter());
                    i += d.len();
                    continue;
                }
            }
        }
        if c == ';' && plain && !is_comment {
            if !cur.trim().is_empty() {
                out.push((start_line, cur.trim().to_string()));
            }
            cur.clear();
            started = false;
            start_line = line;
        } else {
            if !started && !is_comment && !c.is_whitespace() {
                started = true;
                start_line = line;
            }
            cur.push(c);
        }
        i += 1;
    }
    if !cur.trim().is_empty() {
        out.push((start_line, cur.trim().to_string()));
    }
    (out, !in_quote)
}

fn parse_one(sql: &str) -> Result<Vec<Statement>, String> {
    let dialects: [&dyn Dialect; 3] = [&PostgreSqlDialect {}, &MySqlDialect {}, &GenericDialect {}];
    let mut last = String::new();
    for d in dialects {
        match Parser::parse_sql(d, sql) {
            Ok(s) => return Ok(s),
            Err(e) => last = e.to_string(),
        }
    }
    Err(last)
}

fn table_mut<'a>(tables: &'a mut Vec<Table>, name: &str) -> &'a mut Table {
    if let Some(i) = tables.iter().position(|t| t.name == name) {
        return &mut tables[i];
    }
    tables.push(Table {
        name: name.into(),
        columns: vec![],
    });
    tables.last_mut().expect("just pushed")
}

/// Переименовывает таблицу; если такой нет — заводит пустую под новым именем (как `ADD COLUMN`).
/// Прежняя таблица с новым именем (если была) затирается.
fn rename_table(tables: &mut Vec<Table>, old: &str, new: &str) {
    if old == new {
        return;
    }
    match tables.iter().position(|t| t.name == old) {
        Some(i) => {
            tables.retain(|t| t.name != new);
            let i = tables.iter().position(|t| t.name == old).unwrap_or(i);
            tables[i].name = new.into();
        }
        None => {
            table_mut(tables, new);
        }
    }
}

fn column_of(def: &sqlparser::ast::ColumnDef) -> Column {
    let nullable = !def.options.iter().any(|o| {
        matches!(
            o.option,
            ColumnOption::NotNull
                | ColumnOption::Unique {
                    is_primary: true,
                    ..
                }
        )
    });
    Column {
        name: def.name.value.clone(),
        data_type: def.data_type.to_string(),
        nullable,
    }
}

/// Итог применения одного изменения к справочнику таблиц.
#[derive(Debug, PartialEq, Eq)]
enum Applied {
    /// Изменение учтено в справочнике.
    Handled,
    /// Изменение затрагивает таблицы/колонки, но не поддержано: справочник может устареть.
    Unsupported { kind: String, table: String },
    /// Выражение не влияет на справочник таблиц (индексы, функции, DML, `COMMENT` и т.п.).
    Irrelevant,
}

fn unsupported(kind: &str, table: &str) -> Applied {
    Applied::Unsupported {
        kind: kind.into(),
        table: table.into(),
    }
}

/// Предупреждение о неподдержанном изменении таблицы (общий формат для SQL и Liquibase).
fn warn_unsupported(out: &mut ParseResult, path: &str, line: Option<u32>, kind: &str, table: &str) {
    let message = format!("unsupported table change: {kind} on {table}");
    // Несколько одинаковых изменений в одном выражении (`ALTER COLUMN a, ALTER COLUMN b`) — одно предупреждение.
    if out
        .warnings
        .iter()
        .any(|w| w.path == path && w.line == line && w.message == message)
    {
        return;
    }
    out.warn(path, line, message);
}

/// Применяет выражение; результатов может быть несколько (`ALTER TABLE` с несколькими операциями).
fn apply(stmt: &Statement, tables: &mut Vec<Table>) -> Vec<Applied> {
    match stmt {
        Statement::CreateTable(ct) => {
            let t = table_mut(tables, &ct.name.to_string());
            t.columns = ct.columns.iter().map(column_of).collect();
            vec![Applied::Handled]
        }
        Statement::Drop {
            object_type: ObjectType::Table,
            names,
            ..
        } => {
            for n in names {
                let n = n.to_string();
                tables.retain(|t| t.name != n);
            }
            vec![Applied::Handled]
        }
        // MySQL: `RENAME TABLE a TO b, c TO d`.
        Statement::RenameTable(renames) => {
            for r in renames {
                rename_table(tables, &r.old_name.to_string(), &r.new_name.to_string());
            }
            vec![Applied::Handled]
        }
        Statement::AlterTable {
            name, operations, ..
        } => {
            let name = name.to_string();
            operations
                .iter()
                .map(|op| match op {
                    AlterTableOperation::AddColumn { column_def, .. } => {
                        table_mut(tables, &name).columns.push(column_of(column_def));
                        Applied::Handled
                    }
                    AlterTableOperation::DropColumn { column_names, .. } => {
                        if let Some(t) = tables.iter_mut().find(|t| t.name == name) {
                            t.columns
                                .retain(|c| !column_names.iter().any(|n| n.value == c.name));
                        }
                        Applied::Handled
                    }
                    AlterTableOperation::RenameTable { table_name } => {
                        rename_table(tables, &name, &table_name.to_string());
                        Applied::Handled
                    }
                    AlterTableOperation::RenameColumn { .. } => unsupported("RENAME COLUMN", &name),
                    AlterTableOperation::AlterColumn { .. } => unsupported("ALTER COLUMN", &name),
                    AlterTableOperation::ChangeColumn { .. } => unsupported("CHANGE COLUMN", &name),
                    AlterTableOperation::ModifyColumn { .. } => unsupported("MODIFY COLUMN", &name),
                    // Ограничения, партиции и прочее на список таблиц и колонок не влияют.
                    _ => Applied::Irrelevant,
                })
                .collect()
        }
        _ => vec![Applied::Irrelevant],
    }
}

fn parse_sql_text(text: &str, path: &str, out: &mut ParseResult) {
    for (line, stmt) in split_statements(text) {
        match parse_one(&stmt) {
            Ok(statements) => {
                for s in &statements {
                    for applied in apply(s, &mut out.tables) {
                        if let Applied::Unsupported { kind, table } = applied {
                            warn_unsupported(out, path, Some(line), &kind, &table);
                        }
                    }
                }
            }
            Err(e) => out.warn(path, Some(line), format!("statement not parsed: {e}")),
        }
    }
}

fn parse_liquibase_xml(text: &str, path: &str, out: &mut ParseResult) -> Result<(), ParseError> {
    let doc = roxmltree::Document::parse(text).map_err(|e| ParseError {
        message: e.to_string(),
        line: Some(e.pos().row),
    })?;
    for node in doc.descendants().filter(|n| n.is_element()) {
        let line = Some(doc.text_pos_at(node.range().start).row);
        let columns = |n: roxmltree::Node| -> Vec<Column> {
            n.children()
                .filter(|c| c.has_tag_name("column"))
                .map(|c| Column {
                    name: c.attribute("name").unwrap_or("").into(),
                    data_type: c.attribute("type").unwrap_or("").into(),
                    nullable: c
                        .children()
                        .find(|k| k.has_tag_name("constraints"))
                        .map(|k| {
                            k.attribute("nullable") != Some("false")
                                && k.attribute("primaryKey") != Some("true")
                        })
                        .unwrap_or(true),
                })
                .collect()
        };
        match node.tag_name().name() {
            "createTable" => {
                let name = node.attribute("tableName").unwrap_or("");
                table_mut(&mut out.tables, name).columns = columns(node);
            }
            "addColumn" => {
                let name = node.attribute("tableName").unwrap_or("");
                table_mut(&mut out.tables, name)
                    .columns
                    .extend(columns(node));
            }
            "dropColumn" => {
                if let (Some(t), Some(c)) =
                    (node.attribute("tableName"), node.attribute("columnName"))
                    && let Some(t) = out.tables.iter_mut().find(|x| x.name == t)
                {
                    t.columns.retain(|x| x.name != c);
                }
            }
            "dropTable" => match node.attribute("tableName").filter(|n| !n.is_empty()) {
                Some(name) => out.tables.retain(|t| t.name != name),
                None => out.warn(path, line, "dropTable: missing attribute tableName"),
            },
            "renameTable" => match (
                node.attribute("oldTableName").filter(|n| !n.is_empty()),
                node.attribute("newTableName").filter(|n| !n.is_empty()),
            ) {
                (Some(old), Some(new)) => rename_table(&mut out.tables, old, new),
                (None, _) => out.warn(path, line, "renameTable: missing attribute oldTableName"),
                (_, None) => out.warn(path, line, "renameTable: missing attribute newTableName"),
            },
            kind @ ("renameColumn" | "modifyDataType") => warn_unsupported(
                out,
                path,
                line,
                kind,
                node.attribute("tableName").unwrap_or(""),
            ),
            // Встроенный `<sql>` не разбираем: таблица заранее неизвестна.
            "sql" => warn_unsupported(out, path, line, "sql", "<unknown>"),
            "include" | "includeAll" => out.warn(
                path,
                line,
                format!(
                    "{} {}: included files are scanned as separate contracts",
                    node.tag_name().name(),
                    node.attribute("file")
                        .or(node.attribute("path"))
                        .unwrap_or("")
                ),
            ),
            "sqlFile" => out.warn(
                path,
                line,
                format!(
                    "sqlFile {}: add the file as a separate sql contract",
                    node.attribute("path").unwrap_or("")
                ),
            ),
            _ => {}
        }
    }
    Ok(())
}

pub fn parse(input: &ContractInput, opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    parse_onto(input, opts, Vec::new())
}

/// Как `parse`, но изменения применяются к таблицам предыдущих миграций (`base`): так
/// `ALTER`/`DROP` в поздней миграции видят таблицы, созданные ранними. Результат — состояние
/// таблиц после этого файла.
pub fn parse_onto(
    input: &ContractInput,
    _opts: &ParseOptions,
    base: Vec<Table>,
) -> Result<ParseResult, ParseError> {
    let text = std::str::from_utf8(input.bytes).map_err(|e| ParseError {
        message: format!("not UTF-8: {e}"),
        line: None,
    })?;
    // BOM перед первым выражением мешает и SQL-парсеру, и XML.
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut out = ParseResult {
        tables: base,
        ..ParseResult::default()
    };
    let lower = input.path.to_ascii_lowercase();
    if lower.ends_with(".xml") {
        parse_liquibase_xml(text, input.path, &mut out)?;
    } else if lower.ends_with(".yaml") || lower.ends_with(".yml") || lower.ends_with(".json") {
        out.warn(
            input.path,
            None,
            "Liquibase YAML/JSON changelogs are not supported: use XML or formatted SQL",
        );
    } else {
        parse_sql_text(text, input.path, &mut out);
    }
    Ok(out)
}

/// Ссылка Liquibase-changelog на другие файлы миграций.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquibaseInclude {
    /// Путь относительно клона, если задан `relativeToChangelogFile="true"`; иначе — как в файле
    /// (от classpath, вызывающий сопоставляет его с концом пути контракта).
    pub path: String,
    pub relative: bool,
    /// `includeAll`: все файлы каталога по алфавиту.
    pub dir: bool,
}

/// `include`, `includeAll` и `sqlFile` changelog-файла в порядке документа (порядок применения
/// миграций). Файл не XML или не разбирается — пусто: ошибку покажет сам разбор файла.
pub fn liquibase_includes(path: &str, bytes: &[u8]) -> Vec<LiquibaseInclude> {
    if !path.to_ascii_lowercase().ends_with(".xml") {
        return vec![];
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return vec![];
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let Ok(doc) = roxmltree::Document::parse(text) else {
        return vec![];
    };
    doc.descendants()
        .filter(|n| n.is_element())
        .filter_map(|n| {
            let (attr, dir) = match n.tag_name().name() {
                "include" => ("file", false),
                "includeAll" => ("path", true),
                "sqlFile" => ("path", false),
                _ => return None,
            };
            let target = n.attribute(attr).filter(|t| !t.trim().is_empty())?;
            let target = target.trim().replace('\\', "/");
            let relative = n.attribute("relativeToChangelogFile") == Some("true");
            let path = if relative {
                join_path(dir_of(path), &target)
            } else {
                join_path("", target.trim_start_matches("classpath:"))
            };
            Some(LiquibaseInclude {
                path,
                relative,
                dir,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const V1: &str = include_str!("../../../dev/fixtures/flyway/db/migration/V1__init.sql");
    const V2: &str = include_str!("../../../dev/fixtures/flyway/db/migration/V2__add_column.sql");
    const XML: &str = include_str!("../../../dev/fixtures/liquibase/db/changelog/changelog.xml");
    const FORMATTED: &str = include_str!("../../../dev/fixtures/liquibase/db/changelog/002.sql");

    fn run(path: &str, text: &str) -> ParseResult {
        let read = |_: &str| None;
        parse(
            &ContractInput {
                path,
                bytes: text.as_bytes(),
                read: &read,
            },
            &ParseOptions::default(),
        )
        .unwrap()
    }

    fn cols(t: &Table) -> Vec<(&str, &str, bool)> {
        t.columns
            .iter()
            .map(|c| (c.name.as_str(), c.data_type.as_str(), c.nullable))
            .collect()
    }

    #[test]
    fn flyway_create_table_and_dollar_quoted_function() {
        let out = run("db/migration/V1__init.sql", V1);
        assert_eq!(out.tables.len(), 1);
        let orders = &out.tables[0];
        assert_eq!(orders.name, "orders");
        let c = cols(orders);
        assert_eq!(c[0].0, "id");
        assert!(!c[0].2, "primary key не nullable");
        assert_eq!((c[1].0, c[1].2), ("status", false));
        assert_eq!((c[2].0, c[2].2), ("total", true));
        assert!(c[3].1.to_ascii_lowercase().contains("timestamp"));
        assert!(
            out.warnings.len() <= 1,
            "функция с $$ либо разобрана, либо одно предупреждение: {:?}",
            out.warnings
        );
        if let Some(w) = out.warnings.first() {
            assert_eq!(
                w.line,
                Some(11),
                "предупреждение указывает на строку CREATE FUNCTION"
            );
        }
    }

    #[test]
    fn alter_table_add_and_drop_columns() {
        let out = run("db/migration/V2__add_column.sql", V2);
        let orders = out.tables.iter().find(|t| t.name == "orders").unwrap();
        assert_eq!(cols(orders), vec![("note", "TEXT", true)]);
        assert!(out.tables.iter().any(|t| t.name == "audit_log"));
    }

    #[test]
    fn liquibase_xml_and_formatted_sql() {
        let out = run("db/changelog/changelog.xml", XML);
        let inv = out.tables.iter().find(|t| t.name == "invoices").unwrap();
        assert_eq!(
            cols(inv),
            vec![
                ("id", "bigint", false),
                ("amount", "decimal(12,2)", false),
                ("note", "varchar(255)", true),
                ("paid_at", "timestamp", true)
            ]
        );
        let msgs: Vec<&str> = out.warnings.iter().map(|w| w.message.as_str()).collect();
        assert!(
            msgs.iter().any(|m| m.starts_with("sqlFile 002.sql")),
            "{msgs:?}"
        );
        assert!(
            msgs.iter().any(|m| m.starts_with("include other.xml")),
            "{msgs:?}"
        );
        let out = run("db/changelog/002.sql", FORMATTED);
        assert_eq!(out.tables[0].name, "payments");
        assert_eq!(out.tables[0].columns.len(), 3);
        let out = run("db/changelog/x.yaml", "databaseChangeLog: []");
        assert!(out.tables.is_empty() && out.warnings[0].message.contains("not supported"));
    }

    #[test]
    fn later_migrations_apply_onto_earlier_tables() {
        let read = |_: &str| None;
        let v1 = run("db/migration/V1__init.sql", V1);
        let out = parse_onto(
            &ContractInput {
                path: "db/migration/V2__add_column.sql",
                bytes: V2.as_bytes(),
                read: &read,
            },
            &ParseOptions::default(),
            v1.tables,
        )
        .unwrap();
        let orders = out.tables.iter().find(|t| t.name == "orders").unwrap();
        let names: Vec<&str> = orders.columns.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            ["id", "status", "created_at", "note"],
            "total удалён, note добавлен"
        );
        assert!(out.tables.iter().any(|t| t.name == "audit_log"));
    }

    #[test]
    fn liquibase_includes_in_document_order() {
        let inc = liquibase_includes("db/changelog/changelog.xml", XML.as_bytes());
        assert_eq!(
            inc,
            [
                LiquibaseInclude {
                    path: "db/changelog/002.sql".into(),
                    relative: true,
                    dir: false
                },
                LiquibaseInclude {
                    path: "db/changelog/other.xml".into(),
                    relative: true,
                    dir: false
                },
            ]
        );
        let xml = r#"<databaseChangeLog><includeAll path="db/changes/"/><include file="classpath:db/x.xml"/></databaseChangeLog>"#;
        let inc = liquibase_includes("src/main/resources/db/master.xml", xml.as_bytes());
        assert_eq!(
            inc.iter()
                .map(|i| (i.path.as_str(), i.relative, i.dir))
                .collect::<Vec<_>>(),
            [("db/changes", false, true), ("db/x.xml", false, false)]
        );
        assert!(liquibase_includes("a.sql", b"x").is_empty());
        assert!(liquibase_includes("a.xml", b"<broken").is_empty());
    }

    #[test]
    fn splitter_respects_quotes_comments_and_reports_start_lines() {
        let text = "-- a; b\nSELECT ';' AS x; /* c; 'd */ SELECT \"a;b\";\n\n-- ведущий комментарий\nSELECT $$ x; y $$;";
        let parts = split_statements(text);
        let lines: Vec<u32> = parts.iter().map(|(l, _)| *l).collect();
        assert_eq!(lines, [2, 2, 5], "{parts:?}");
        assert!(parts[2].1.ends_with("$$ x; y $$"));
        assert_eq!(parts.len(), 3);
    }

    #[test]
    fn bad_statement_warns_with_its_line_and_does_not_stop() {
        let out = run(
            "m.sql",
            "CREATE TABLE a (id int);\n\nTHIS IS NOT SQL;\nCREATE TABLE b (id int);",
        );
        assert_eq!(
            out.tables
                .iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert_eq!(out.warnings[0].line, Some(3));
    }

    fn names(out: &ParseResult) -> Vec<&str> {
        out.tables.iter().map(|t| t.name.as_str()).collect()
    }

    #[test]
    fn drop_table_and_rename_table_update_the_list() {
        let out = run(
            "m.sql",
            "CREATE TABLE a (x int);\nCREATE TABLE c (y int);\nDROP TABLE c;\nALTER TABLE a RENAME TO b;\nDROP TABLE IF EXISTS nope;",
        );
        assert_eq!(names(&out), ["b"]);
        assert_eq!(cols(&out.tables[0]), vec![("x", "INT", true)]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
    }

    #[test]
    fn unsupported_table_changes_warn_with_line() {
        let out = run(
            "m.sql",
            "CREATE TABLE a (x int);\n\nALTER TABLE a ALTER COLUMN x TYPE text;",
        );
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert_eq!(out.warnings[0].line, Some(3));
        let m = &out.warnings[0].message;
        assert!(
            m.starts_with("unsupported table change:") && m.ends_with(" on a"),
            "{m}"
        );
        let out = run(
            "m.sql",
            "CREATE TABLE a (x int);\nALTER TABLE a RENAME COLUMN x TO y;",
        );
        assert_eq!(out.warnings.len(), 1);
        assert_eq!(out.warnings[0].line, Some(2));
    }

    #[test]
    fn irrelevant_statements_do_not_warn() {
        let out = run(
            "m.sql",
            "CREATE TABLE a (x int);\nCREATE INDEX i ON a (x);\nINSERT INTO a VALUES (1);\nUPDATE a SET x = 2;\nCOMMENT ON TABLE a IS 'c';\nTRUNCATE a;",
        );
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        assert_eq!(names(&out), ["a"]);
    }

    #[test]
    fn liquibase_drop_rename_and_unsupported_changes() {
        let xml = r#"<databaseChangeLog>
  <changeSet id="1" author="d">
    <createTable tableName="a"><column name="id" type="int"/></createTable>
    <createTable tableName="c"><column name="id" type="int"/></createTable>
  </changeSet>
  <changeSet id="2" author="d">
    <dropTable tableName="c"/>
    <renameTable oldTableName="a" newTableName="b"/>
    <modifyDataType tableName="b" columnName="id" newDataType="bigint"/>
  </changeSet>
</databaseChangeLog>"#;
        let out = run("changelog.xml", xml);
        assert_eq!(names(&out), ["b"]);
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
        assert_eq!(out.warnings[0].line, Some(9));
        assert_eq!(
            out.warnings[0].message,
            "unsupported table change: modifyDataType on b"
        );
    }

    #[test]
    fn splitter_handles_tagged_dollar_quotes() {
        let parts = split_statements("DO $BODY$ a; b $BODY$; SELECT 1;");
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert!(parts[0].1.ends_with("$BODY$"));
        let parts = split_statements("SELECT $do$ x; $$ y; $do$; SELECT $1;");
        assert_eq!(parts.len(), 2, "{parts:?}");
    }

    #[test]
    fn splitter_handles_backslash_quotes_and_backticks() {
        // Файл целиком сбалансирован и при буквальном правиле, но режет `'it\\'s` неверно, поэтому
        // MySQL-вариант проверяется отдельно (`mysql_style_backslash_falls_back_to_escape_rule`).
        let parts = split_statements("SELECT E'a\\'b;c'; SELECT `a;b`; SELECT 'it\\'s; x';");
        assert_eq!(parts.len(), 3, "{parts:?}");
    }

    #[test]
    fn mysql_style_backslash_falls_back_to_escape_rule() {
        // Буквальное правило оставляет кавычку открытой (`'it\'` закрывается, `x'` открывает новую),
        // поэтому применяется правило с экранированием.
        let parts = split_statements("SELECT 'it\\'s; x'; SELECT 1;");
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert_eq!(parts[0].1, "SELECT 'it\\'s; x'");
        assert_eq!(parts[1].1, "SELECT 1");
    }

    #[test]
    fn plain_string_backslash_is_literal_in_postgres() {
        let out = run(
            "m.sql",
            "INSERT INTO t VALUES ('C:\\'); CREATE TABLE u (id int);",
        );
        assert_eq!(names(&out), ["u"]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let parts = split_statements("INSERT INTO t VALUES ('C:\\'); CREATE TABLE u (id int);");
        assert_eq!(parts.len(), 2, "{parts:?}");
        let parts = split_statements("SELECT regexp_replace(x, '\\', ''); SELECT 2;");
        assert_eq!(parts.len(), 2, "{parts:?}");
    }

    #[test]
    fn estring_backslash_escapes_quote() {
        let parts = split_statements("SELECT E'a\\'b;c'; SELECT 2;");
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert_eq!(parts[0].1, "SELECT E'a\\'b;c'");
        // `E` как хвост идентификатора не делает строку E-строкой.
        let parts = split_statements("SELECT name'a\\'; SELECT 2;");
        assert_eq!(parts.len(), 2, "{parts:?}");
    }

    #[test]
    fn dollar_quote_closes_when_glued_to_identifier() {
        let out = run("m.sql", "DO $$ BEGIN x; END$$; CREATE TABLE t (id int);");
        assert_eq!(names(&out), ["t"]);
        let parts = split_statements("DO $$ BEGIN x; END$$; CREATE TABLE t (id int);");
        assert_eq!(parts.len(), 2, "{parts:?}");
        let parts = split_statements("DO $body$ BEGIN x; END$body$; CREATE TABLE t (id int);");
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert!(parts[0].1.ends_with("END$body$"));
        let out = run(
            "m.sql",
            "DO $body$ BEGIN x; END$body$; CREATE TABLE t (id int);",
        );
        assert_eq!(names(&out), ["t"]);
    }

    #[test]
    fn unclosed_dollar_quote_flushes_tail_at_eof() {
        let parts = split_statements("SELECT 1; DO $$ BEGIN x; END");
        assert_eq!(parts.len(), 2, "{parts:?}");
        assert_eq!(parts[1].1, "DO $$ BEGIN x; END");
    }

    #[test]
    fn liquibase_missing_names_are_skipped_with_warning() {
        let xml = r#"<databaseChangeLog>
  <changeSet id="1" author="d">
    <createTable tableName="a"><column name="id" type="int"/></createTable>
    <dropTable/>
    <renameTable oldTableName="a"/>
    <renameTable newTableName="z"/>
  </changeSet>
</databaseChangeLog>"#;
        let out = run("changelog.xml", xml);
        assert_eq!(names(&out), ["a"]);
        assert_eq!(out.warnings.len(), 3, "{:?}", out.warnings);
        assert_eq!(out.warnings[0].line, Some(4));
        assert!(
            out.warnings
                .iter()
                .all(|w| w.message.contains("missing attribute"))
        );
    }

    #[test]
    fn identical_unsupported_warnings_on_one_line_collapse() {
        let out = run(
            "m.sql",
            "CREATE TABLE a (x int, y int);\nALTER TABLE a ALTER COLUMN x TYPE text, ALTER COLUMN y TYPE text;",
        );
        assert_eq!(out.warnings.len(), 1, "{:?}", out.warnings);
    }

    #[test]
    fn bom_prefixed_file_parses_first_statement() {
        let out = run("m.sql", "\u{feff}CREATE TABLE a (id int);");
        assert_eq!(names(&out), ["a"]);
        assert!(out.warnings.is_empty(), "{:?}", out.warnings);
        let parts = split_statements("\u{feff}SELECT 1;");
        assert_eq!(parts[0].1, "SELECT 1");
    }
}
