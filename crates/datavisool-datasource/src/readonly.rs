/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::ops::ControlFlow;

use sqlparser::ast::{
    Expr, FunctionArg, FunctionArgExpr, Query, Select, Statement, TableFactor, Visit, Visitor,
};
use sqlparser::dialect::{Dialect, MsSqlDialect, OracleDialect};
use sqlparser::keywords::Keyword;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, Tokenizer};

#[path = "readonly_compat.rs"]
mod compatibility;

/// Returns whether `sql` is a single read-only Oracle or SQL Server query.
///
/// This is a syntactic safety gate, not a substitute for database permissions:
/// user-defined SQL functions can have database-specific side effects that a
/// dialect parser cannot determine. The database account used for queries must
/// still have read-only privileges.
pub fn is_readonly_query(database_type: &str, sql: &str) -> bool {
    let database_type = database_type.trim();
    if sql.trim().is_empty() || sql.len() > MAX_SQL_BYTES {
        return false;
    }

    match database_type.to_ascii_lowercase().as_str() {
        "oracle" => parse_readonly(&OracleDialect {}, sql, false),
        "sqlserver" | "mssql" => parse_readonly(&MsSqlDialect {}, sql, true),
        _ => false,
    }
}

const MAX_SQL_BYTES: usize = 1024 * 1024;
// The parser's recursion limit does not bound left-associated expressions.
// Bound the tree before constructing it, so parsing, visiting and dropping it
// all fit on the host's ordinary async worker stack.
const MAX_SQL_TOKENS: usize = 512;
// sqlparser is built without stack-growing recursive protection. Its default
// depth of 50 is too high for nested function calls on a 2 MiB Tokio worker.
const MAX_PARSE_RECURSION: usize = 16;

fn parse_readonly(dialect: &dyn Dialect, sql: &str, is_sql_server: bool) -> bool {
    let Ok(mut tokens) = Tokenizer::new(dialect, sql).tokenize_with_location() else {
        return false;
    };
    tokens.retain(|token| !matches!(token.token, Token::Whitespace(_)));
    if tokens.len() > MAX_SQL_TOKENS {
        return false;
    }
    // NEXT VALUE FOR changes persistent SQL Server sequence state. Match real
    // keyword tokens instead of recursively formatting every expression; text
    // inside literals, quoted identifiers and comments is not SQL syntax.
    if tokens.windows(3).any(|window| {
        window.iter().zip([Keyword::NEXT, Keyword::VALUE, Keyword::FOR]).all(
            |(token, keyword)| matches!(&token.token, Token::Word(word) if word.keyword == keyword),
        )
    }) {
        return false;
    }

    let flashback_expressions = if is_sql_server {
        if !compatibility::strip_sql_server_maxdop_hint(&mut tokens) {
            return false;
        }
        Vec::new()
    } else {
        let Some(expressions) =
            compatibility::extract_oracle_flashback(&mut tokens, MAX_PARSE_RECURSION)
        else {
            return false;
        };
        expressions
    };

    let mut parser = Parser::new(dialect)
        .with_recursion_limit(MAX_PARSE_RECURSION)
        .with_tokens_with_locations(tokens);
    let Ok(statements) = parser.parse_statements() else {
        return false;
    };
    let [Statement::Query(query)] = statements.as_slice() else {
        return false;
    };

    let mut visitor = ReadOnlyVisitor { is_sql_server };
    matches!(query.visit(&mut visitor), ControlFlow::Continue(()))
        && flashback_expressions
            .iter()
            .all(|expression| matches!(expression.visit(&mut visitor), ControlFlow::Continue(())))
}

struct ReadOnlyVisitor {
    is_sql_server: bool,
}

impl Visitor for ReadOnlyVisitor {
    type Break = ();

    fn pre_visit_statement(&mut self, statement: &Statement) -> ControlFlow<Self::Break> {
        // A query AST can contain DML bodies (for example, a data-modifying
        // CTE). Visit is recursive, so reject every nested non-query statement.
        if matches!(statement, Statement::Query(_)) {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    }

    fn pre_visit_query(&mut self, query: &Query) -> ControlFlow<Self::Break> {
        // sqlparser represents FOR UPDATE / FOR SHARE here. MSSQL's FOR XML
        // and FOR JSON are output formatting clauses and do not acquire locks.
        if query.locks.is_empty() {
            ControlFlow::Continue(())
        } else {
            ControlFlow::Break(())
        }
    }

    fn pre_visit_select(&mut self, select: &Select) -> ControlFlow<Self::Break> {
        if select.into.is_none() {
            ControlFlow::Continue(())
        } else {
            // SELECT INTO creates/populates a table in both supported dialects.
            ControlFlow::Break(())
        }
    }

    fn pre_visit_table_factor(&mut self, table_factor: &TableFactor) -> ControlFlow<Self::Break> {
        // MSSQL table hints can request update/exclusive locks (UPDLOCK,
        // XLOCK, HOLDLOCK). Reject hints as a class so new lock hints fail closed.
        if matches!(table_factor, TableFactor::Table { with_hints, .. } if !with_hints.is_empty())
            || (self.is_sql_server && is_legacy_sql_server_table_hint(table_factor))
        {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }

    fn pre_visit_expr(&mut self, expr: &Expr) -> ControlFlow<Self::Break> {
        // Oracle sequence NEXTVAL changes persistent state even inside SELECT.
        // CURRVAL remains a read and is permitted.
        let oracle_nextval = matches!(expr, Expr::CompoundIdentifier(parts)
            if is_oracle_nextval(parts));
        if oracle_nextval {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    }
}

fn is_oracle_nextval(parts: &[sqlparser::ast::Ident]) -> bool {
    // sqlparser does not resolve identifiers against Oracle schema objects,
    // so keep rejecting a final NEXTVAL component even when it is quoted.
    parts.last().is_some_and(is_oracle_nextval_part) || parts.iter().any(is_remote_oracle_nextval)
}

fn is_oracle_nextval_part(part: &sqlparser::ast::Ident) -> bool {
    part.value.eq_ignore_ascii_case("NEXTVAL")
}

fn is_remote_oracle_nextval(part: &sqlparser::ast::Ident) -> bool {
    // Oracle's remote syntax appends @dblink to the unquoted NEXTVAL token;
    // sqlparser keeps them together (and may parse a quoted dotted link as a
    // select-item alias).
    if part.quote_style.is_some() {
        return false;
    }
    part.value
        .get(.."NEXTVAL@".len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("NEXTVAL@"))
}

fn is_legacy_sql_server_table_hint(table_factor: &TableFactor) -> bool {
    let TableFactor::Table {
        args: Some(args), ..
    } = table_factor
    else {
        return false;
    };

    let [FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Identifier(identifier)))] =
        args.args.as_slice()
    else {
        return false;
    };
    if identifier.quote_style.is_some() {
        return false;
    }

    // These SQL Server table hints may be written without WITH and are parsed
    // as table-valued-function arguments by sqlparser. Matching only this
    // single bare identifier keeps ordinary TVF calls and expression args
    // available while rejecting the legacy locking-hint form.
    const LEGACY_HINTS: &[&str] = &[
        "NOLOCK",
        "READUNCOMMITTED",
        "UPDLOCK",
        "HOLDLOCK",
        "READCOMMITTEDLOCK",
        "REPEATABLEREAD",
        "SERIALIZABLE",
        "READCOMMITTED",
        "TABLOCK",
        "TABLOCKX",
        "PAGLOCK",
        "ROWLOCK",
        "NOWAIT",
        "READPAST",
        "XLOCK",
        "SNAPSHOT",
        "NOEXPAND",
    ];
    LEGACY_HINTS
        .iter()
        .any(|hint| identifier.value.eq_ignore_ascii_case(hint))
}

#[cfg(test)]
mod tests {
    use super::{compatibility, is_readonly_query, MsSqlDialect, Token, Tokenizer};

    #[test]
    fn accepts_simple_selects_and_read_only_ctes_for_both_dialects() {
        assert!(is_readonly_query("oracle", "SELECT 1 FROM dual"));
        assert!(is_readonly_query("mssql", "SELECT 1"));
        assert!(is_readonly_query(
            "oracle",
            "WITH items AS (SELECT 1 AS id FROM dual) SELECT id FROM items"
        ));
        assert!(is_readonly_query(
            "sqlserver",
            "WITH items AS (SELECT 1 AS id) SELECT id FROM items"
        ));
    }

    #[test]
    fn treats_semicolons_inside_string_literals_as_data() {
        assert!(is_readonly_query(
            "oracle",
            "SELECT '; DELETE FROM users' AS message FROM dual"
        ));
        assert!(is_readonly_query(
            "mssql",
            "SELECT '; DELETE FROM users' AS message"
        ));
    }

    #[test]
    fn accepts_comments_around_a_read_only_query() {
        assert!(is_readonly_query(
            "oracle",
            "/* report */ SELECT 1 FROM dual -- end of query"
        ));
        assert!(is_readonly_query("mssql", "-- report\nSELECT 1"));
    }

    #[test]
    fn rejects_empty_comments_and_unsupported_database_types() {
        assert!(!is_readonly_query("oracle", ""));
        assert!(!is_readonly_query(
            "oracle",
            " /* only a comment */ -- still empty"
        ));
        assert!(!is_readonly_query("sqlite", "SELECT 1"));
    }

    #[test]
    fn rejects_multiple_statements_and_non_query_statements() {
        assert!(!is_readonly_query("mssql", "SELECT 1; DELETE FROM users"));
        assert!(!is_readonly_query("oracle", "DELETE FROM users"));
        assert!(!is_readonly_query("mssql", "EXEC xp_cmdshell 'whoami'"));
    }

    #[test]
    fn rejects_data_modifying_ctes() {
        // Some databases reject this syntax before execution; if accepted by
        // the parser, the visitor must still reject the nested DELETE node.
        assert!(!is_readonly_query(
            "mssql",
            "WITH removed AS (DELETE FROM users OUTPUT DELETED.id) SELECT id FROM removed"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "WITH removed AS (DELETE FROM users RETURNING id) SELECT id FROM removed"
        ));
    }

    #[test]
    fn rejects_select_into_and_locking_reads() {
        assert!(!is_readonly_query(
            "mssql",
            "SELECT id INTO copied FROM users"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT id FROM users FOR UPDATE"
        ));
        assert!(!is_readonly_query(
            "mssql",
            "SELECT id FROM users WITH (UPDLOCK)"
        ));
    }

    #[test]
    fn rejects_locks_inside_nested_queries_and_oracle_sequence_increments() {
        assert!(!is_readonly_query(
            "oracle",
            "SELECT id FROM users WHERE id IN (SELECT id FROM users FOR UPDATE)"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT users_seq.NEXTVAL FROM dual"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT users_seq.CURRVAL FROM dual"
        ));
        assert!(!is_readonly_query(
            "mssql",
            "SELECT NEXT VALUE FOR dbo.users_seq"
        ));
    }

    #[test]
    fn rejects_remote_oracle_nextval_and_preserves_quoted_identifier_policy() {
        assert!(!is_readonly_query(
            "oracle",
            "SELECT users_seq.NEXTVAL@remote_db FROM dual"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT app.users_seq.nExTvAl@REMOTE_DB FROM dual"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT \"App\".\"Seq\".NEXTVAL@\"Remote.Link\" FROM dual"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT \"App\".\"Seq\".NEXTVAL FROM dual"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT app.seq.NEXTVAL@remote.db FROM dual"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT users_seq.CURRVAL@remote_db FROM dual"
        ));
        assert!(is_readonly_query("oracle", "SELECT \"NEXTVAL\" FROM dual"));
        assert!(is_readonly_query(
            "oracle",
            "SELECT t.\"NEXTVAL@remote\" FROM dual t"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT t.\"NEXTVAL\" FROM dual t"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT 'seq.NEXTVAL@remote_db' AS label FROM dual"
        ));
    }

    #[test]
    fn rejects_legacy_sql_server_lock_hints_without_rejecting_table_functions() {
        for hint in [
            "UPDLOCK",
            "XLOCK",
            "TABLOCKX",
            "HOLDLOCK",
            "READCOMMITTEDLOCK",
            "NOLOCK",
            "READUNCOMMITTED",
            "REPEATABLEREAD",
            "SERIALIZABLE",
            "READCOMMITTED",
            "TABLOCK",
            "PAGLOCK",
            "ROWLOCK",
            "NOWAIT",
            "READPAST",
            "SNAPSHOT",
            "NOEXPAND",
        ] {
            let sql = format!("SELECT id FROM users ({hint})");
            assert!(!is_readonly_query("mssql", &sql), "accepted {sql}");
        }

        assert!(!is_readonly_query(
            "mssql",
            "SELECT id FROM users WITH (UPDLOCK)"
        ));
        assert!(is_readonly_query(
            "mssql",
            "SELECT value FROM STRING_SPLIT('a,b', ',')"
        ));
        assert!(is_readonly_query(
            "mssql",
            "SELECT value FROM dbo.get_values(@user_id)"
        ));
        assert!(is_readonly_query(
            "mssql",
            "SELECT value FROM dbo.get_values(UPDLOCK + 1)"
        ));
        assert!(is_readonly_query(
            "mssql",
            "SELECT f.value FROM dbo.users AS u CROSS APPLY dbo.get_values(u.UPDLOCK) AS f"
        ));
        assert!(is_readonly_query(
            "mssql",
            "SELECT f.value FROM dbo.users AS u CROSS APPLY dbo.get_values([UPDLOCK]) AS f"
        ));
    }

    #[test]
    fn accepts_sql_server_maxdop_and_oracle_flashback_reads() {
        assert!(is_readonly_query(
            "mssql",
            "SELECT id FROM users OPTION (MAXDOP 1)"
        ));
        // The old parser accepts this by interpreting OPTION(...) as a table
        // alias and column list. Preserve that behavior for non-MAXDOP hints.
        assert!(is_readonly_query(
            "mssql",
            "SELECT id FROM users OPTION (RECOMPILE)"
        ));
        assert!(is_readonly_query(
            "sqlserver",
            "SELECT id FROM users OPTION(MAXDOP 32767);"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF SCN 123"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF SCN 123 + 1"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF SCN TO_NUMBER('123')"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF TIMESTAMP SYSTIMESTAMP"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF TIMESTAMP TO_TIMESTAMP('2026-01-01', 'YYYY-MM-DD') e"
        ));
    }

    #[test]
    fn oracle_ordinary_aliases_keep_native_dialect_behavior() {
        for alias in [
            "changes",
            "at",
            "before",
            "version",
            "timestamp",
            "system_time",
        ] {
            let sql = format!("SELECT {alias}.id FROM employees {alias}");
            assert!(is_readonly_query("oracle", &sql), "rejected {sql}");
        }
        assert!(is_readonly_query(
            "oracle",
            "SELECT created_at AT TIME ZONE 'UTC' AS created FROM employees at"
        ));
        assert!(is_readonly_query(
            "oracle",
            "SELECT 'AS OF SCN 123' AS label FROM employees changes"
        ));
    }

    #[test]
    fn oracle_flashback_keeps_aliases_joins_and_filters() {
        for alias in ["changes", "at", "before"] {
            for version in ["SCN 123 + 1", "TIMESTAMP SYSTIMESTAMP"] {
                let sql = format!(
                    "SELECT {alias}.id FROM employees AS OF {version} {alias} WHERE {alias}.id > 0"
                );
                assert!(is_readonly_query("oracle", &sql), "rejected {sql}");
            }
        }
        assert!(is_readonly_query(
            "oracle",
            "SELECT changes.id FROM employees AS OF SCN 123 changes JOIN departments AS OF TIMESTAMP SYSTIMESTAMP at ON changes.department_id = at.id"
        ));
    }

    #[test]
    fn oracle_flashback_extraction_does_not_hide_unsafe_sql() {
        for sql in [
            "SELECT id FROM employees AS OF SCN 123 changes FOR UPDATE",
            "SELECT id INTO copied FROM employees AS OF SCN 123 changes",
            "SELECT id FROM employees AS OF SCN users_seq.NEXTVAL changes",
            "SELECT id FROM employees AS OF TIMESTAMP users_seq.NEXTVAL@remote_db before",
            "SELECT id FROM employees AS OF SCN 123; DELETE FROM employees",
            "SELECT id FROM employees AS OF SCN 123 WHERE id IN (SELECT id FROM other FOR UPDATE)",
            "SELECT id FROM employees AS OF SCN (SELECT users_seq.NEXTVAL FROM dual)",
            "SELECT id FROM employees AS OF SCN",
            "SELECT id FROM employees AS OF SCN ; DELETE FROM employees",
        ] {
            assert!(!is_readonly_query("oracle", sql), "accepted {sql}");
        }
    }

    #[test]
    fn rejects_unsafe_or_malformed_sql_server_maxdop_combinations() {
        for sql in [
            "SELECT id FROM users OPTION (MAXDOP 1, RECOMPILE)",
            "SELECT id FROM users OPTION (RECOMPILE, MAXDOP 1)",
            "SELECT id FROM users OPTION (MAXDOP @degree)",
            "SELECT id FROM users OPTION MAXDOP 1",
            "SELECT id FROM users OPTION (MAXDOP 32768)",
            "SELECT id FROM users; OPTION (MAXDOP 1)",
            "SELECT id FROM users OPTION (MAXDOP 1); DELETE FROM users",
            "SELECT NEXT VALUE FOR users_seq OPTION (MAXDOP 1)",
            "SELECT id INTO copied FROM users OPTION (MAXDOP 1)",
            "SELECT id FROM users WITH (UPDLOCK) OPTION (MAXDOP 1)",
        ] {
            assert!(!is_readonly_query("mssql", sql), "accepted {sql}");
        }
    }

    #[test]
    fn oracle_flashback_does_not_hide_sequence_or_locking_expressions() {
        assert!(!is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF SCN users_seq.NEXTVAL"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF TIMESTAMP users_seq.NEXTVAL"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF SCN users_seq.NEXTVAL@remote_db"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF SCN (SELECT users_seq.NEXTVAL FROM dual)"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT * FROM employees AS OF SCN 123 FOR UPDATE"
        ));
        assert!(!is_readonly_query(
            "oracle",
            "SELECT id FROM employees AS OF SCN 123 INTO copied"
        ));
    }

    #[test]
    fn does_not_treat_nested_or_quoted_maxdop_as_a_query_hint_name() {
        let dialect = MsSqlDialect {};
        for sql in [
            "SELECT id FROM users OPTION (TABLE HINT(MAXDOP, INDEX(ix_users)))",
            "SELECT id FROM users OPTION ('MAXDOP')",
            "SELECT id FROM users OPTION ([MAXDOP])",
        ] {
            let mut tokens = Tokenizer::new(&dialect, sql)
                .tokenize_with_location()
                .unwrap();
            tokens.retain(|token| !matches!(token.token, Token::Whitespace(_)));
            let before = tokens.clone();
            assert!(compatibility::strip_sql_server_maxdop_hint(&mut tokens));
            assert_eq!(tokens, before, "mutated {sql}");
        }
    }

    #[test]
    fn rejects_sql_beyond_the_input_size_limit() {
        let sql = format!("SELECT '{}'", "x".repeat(1024 * 1024));
        assert!(!is_readonly_query("mssql", &sql));
    }

    #[test]
    fn token_limit_preserves_literals_and_comments() {
        let at_limit = format!("SELECT {}", vec!["1"; 256].join(" + "));
        assert!(is_readonly_query("mssql", &at_limit));
        let over_limit = format!("{at_limit} + 1");
        assert!(!is_readonly_query("mssql", &over_limit));
        let long_literal = format!("SELECT '{}'", "next value for ; + ".repeat(1000));
        assert!(is_readonly_query("mssql", &long_literal));
        let long_comment = format!("/* {} */ SELECT 1", "next value for ".repeat(1000));
        assert!(is_readonly_query("mssql", &long_comment));
        assert!(is_readonly_query(
            "mssql",
            "SELECT [next value for] AS [sequence label]"
        ));
        assert!(!is_readonly_query(
            "mssql",
            "SELECT NEXT /* comment */ VALUE FOR dbo.users_seq"
        ));
        assert!(is_readonly_query("mssql", "SELECT ABS(ABS(ABS(1)))"));
    }

    #[test]
    fn complex_queries_do_not_abort_async_workers() {
        // A stack overflow aborts the entire process; isolate this regression
        // so a failure is reported as a failed test rather than losing the suite.
        const CHILD: &str = "DATAVISOOL_DATASOURCE_READONLY_STACK_TEST_CHILD";
        if std::env::var_os(CHILD).is_none() {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "readonly::tests::complex_queries_do_not_abort_async_workers",
                    "--nocapture",
                ])
                .env(CHILD, "1")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "worker regression failed: {}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
            return;
        }

        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .build()
            .unwrap()
            .block_on(async {
                tokio::spawn(async {
                    for (index, sql) in [
                        format!("SELECT {}", vec!["1"; 1000].join(" + ")),
                        format!("SELECT {}", vec!["1"; 200_000].join(" + ")),
                        format!("SELECT 1 WHERE {}", vec!["1 = 1"; 1000].join(" AND ")),
                        vec!["SELECT 1"; 1000].join(" UNION ALL "),
                        format!("SELECT {}1{}", "ABS(".repeat(100), ")".repeat(100)),
                        format!("SELECT {}1{}", "(".repeat(200), ")".repeat(200)),
                        format!("SELECT {}1", "ABS(".repeat(100)),
                        format!("SELECT {}", vec!["CASE WHEN 1=1 THEN"; 30].join(" ")),
                        format!("SELECT {} +", vec!["1"; 255].join(" + ")),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        eprintln!("checking complex query {index}");
                        assert!(!is_readonly_query("mssql", &sql));
                        assert!(!is_readonly_query("oracle", &sql));
                    }
                    // Exercise traversal and destruction at the accepted limit
                    // on the same worker stack as the formerly crashing input.
                    let sql = format!("SELECT {}", vec!["1"; 256].join(" + "));
                    assert!(is_readonly_query("mssql", &sql));
                    let sql = format!("SELECT {} FROM dual", vec!["1"; 255].join(" + "));
                    assert!(is_readonly_query("oracle", &sql));
                    assert!(is_readonly_query(
                        "mssql",
                        &vec!["SELECT 1"; 128].join(" UNION ALL ")
                    ));
                })
                .await
                .unwrap();
            });
    }
}
