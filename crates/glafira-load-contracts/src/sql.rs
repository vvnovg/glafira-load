//! SQL-миграции → справочник таблиц (спека L1 §5.2, решение плана 10 про Liquibase).
//! Операций `sql` не создаёт: их задаёт человек, а таблицы нужны как подсказки.

use sqlparser::ast::{AlterTableOperation, ColumnOption, Statement};
use sqlparser::dialect::{Dialect, GenericDialect, MySqlDialect, PostgreSqlDialect};
use sqlparser::parser::Parser;

use crate::{Column, ContractInput, ParseError, ParseOptions, ParseResult, Table};

/// Разбивает текст на выражения по `;` вне кавычек (`'…'`, `"…"`), `$$ … $$` и комментариев
/// (`-- …`, `/* … */`), с номером строки начала выражения (первый значащий символ).
fn split_statements(text: &str) -> Vec<(u32, String)> {
    let mut out = Vec::new();
    let (mut cur, mut start_line, mut line) = (String::new(), 1u32, 1u32);
    let (mut in_quote, mut in_ident, mut in_dollar) = (false, false, false);
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
        let plain = !in_quote && !in_ident && !in_dollar && !in_comment && !in_block;
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
            if c == '\'' && !in_dollar && !in_ident {
                in_quote = !in_quote;
            } else if c == '"' && !in_dollar && !in_quote {
                in_ident = !in_ident;
            } else if c == '$' && next == Some('$') && !in_quote && !in_ident {
                in_dollar = !in_dollar;
                cur.push('$');
                i += 1;
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
    out
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

fn apply(stmt: &Statement, tables: &mut Vec<Table>) -> bool {
    match stmt {
        Statement::CreateTable(ct) => {
            let t = table_mut(tables, &ct.name.to_string());
            t.columns = ct.columns.iter().map(column_of).collect();
            true
        }
        Statement::AlterTable {
            name, operations, ..
        } => {
            for op in operations {
                match op {
                    AlterTableOperation::AddColumn { column_def, .. } => {
                        table_mut(tables, &name.to_string())
                            .columns
                            .push(column_of(column_def))
                    }
                    AlterTableOperation::DropColumn { column_names, .. } => {
                        if let Some(t) = tables.iter_mut().find(|t| t.name == name.to_string()) {
                            t.columns
                                .retain(|c| !column_names.iter().any(|n| n.value == c.name));
                        }
                    }
                    _ => {}
                }
            }
            true
        }
        _ => false,
    }
}

fn parse_sql_text(text: &str, path: &str, out: &mut ParseResult) {
    for (line, stmt) in split_statements(text) {
        match parse_one(&stmt) {
            Ok(statements) => {
                for s in &statements {
                    apply(s, &mut out.tables);
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

pub fn parse(input: &ContractInput, _opts: &ParseOptions) -> Result<ParseResult, ParseError> {
    let text = std::str::from_utf8(input.bytes).map_err(|e| ParseError {
        message: format!("not UTF-8: {e}"),
        line: None,
    })?;
    let mut out = ParseResult::default();
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
}
