/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use sqlparser::ast::Expr;
use sqlparser::dialect::OracleDialect;
use sqlparser::keywords::Keyword;
use sqlparser::parser::Parser;
use sqlparser::tokenizer::{Token, TokenWithSpan};

/// Extracts only Oracle AS OF expressions for a separate read-only AST visit.
/// The main query keeps the ordinary Oracle dialect, including its alias rules.
/// This changes validation tokens only; callers execute the original SQL.
pub(super) fn extract_oracle_flashback(
    tokens: &mut Vec<TokenWithSpan>,
    recursion_limit: usize,
) -> Option<Vec<Expr>> {
    let mut expressions = Vec::new();
    // Work inside-out so a nested query's flashback clause is handled before
    // parsing any containing expression. The original token budget is already
    // checked by the caller and bounds both these scans and expression parsing.
    while let Some(index) = tokens.windows(3).rposition(|window| {
        is_word(&window[0].token, Keyword::AS, "AS")
            && is_word(&window[1].token, Keyword::OF, "OF")
            && (is_unquoted_word(&window[2].token, "SCN")
                || is_word(&window[2].token, Keyword::TIMESTAMP, "TIMESTAMP"))
    }) {
        let expression_start = index + 3;
        let dialect = OracleDialect {};
        let mut parser = Parser::new(&dialect)
            .with_recursion_limit(recursion_limit)
            .with_tokens_with_locations(tokens[expression_start..].to_vec());
        let expression = parser.parse_expr().ok()?;
        let consumed = parser.index();
        if consumed == 0 || consumed > tokens.len() - expression_start {
            return None;
        }
        // Parser::index points to the first unconsumed token: aliases, JOIN,
        // WHERE, locks and subsequent statements remain in the main query.
        tokens.drain(index..expression_start + consumed);
        expressions.push(expression);
    }
    Some(expressions)
}

/// Removes a single trailing SQL Server `OPTION (MAXDOP n)` hint after
/// validating the exact token shape. Other query hints stay on the existing
/// parser/visitor path.
pub(super) fn strip_sql_server_maxdop_hint(tokens: &mut Vec<TokenWithSpan>) -> bool {
    let content_end = if tokens
        .last()
        .is_some_and(|token| matches!(token.token, Token::SemiColon))
    {
        tokens.len() - 1
    } else {
        tokens.len()
    };

    let mut depth = 0usize;
    let mut has_top_level_semicolon = false;
    for index in 0..content_end {
        if depth == 0
            && is_unquoted_word(&tokens[index].token, "OPTION")
            && matches!(
                tokens.get(index + 1).map(|token| &token.token),
                Some(Token::LParen)
            )
            && parenthesized_suffix_ends_at(tokens, index + 1, content_end)
        {
            // Preserve historical parser/visitor behavior for other query
            // hints. Only MAXDOP receives the token-level compatibility path.
            if !contains_maxdop_hint_name(tokens, index, content_end) {
                return true;
            }
            if has_top_level_semicolon {
                return false;
            }
            if !is_maxdop_suffix(tokens, index, content_end) {
                return false;
            }
            tokens.drain(index..content_end);
            return true;
        }

        match &tokens[index].token {
            Token::LParen => depth = depth.saturating_add(1),
            Token::RParen if depth > 0 => depth -= 1,
            Token::SemiColon if depth == 0 => has_top_level_semicolon = true,
            _ => {}
        }
    }
    true
}

fn is_maxdop_suffix(tokens: &[TokenWithSpan], option_index: usize, content_end: usize) -> bool {
    let Some(maxdop_index) = option_index.checked_add(2) else {
        return false;
    };
    let Some(value_index) = option_index.checked_add(3) else {
        return false;
    };
    let Some(close_index) = option_index.checked_add(4) else {
        return false;
    };
    if close_index + 1 != content_end
        || !matches!(
            tokens.get(close_index).map(|token| &token.token),
            Some(Token::RParen)
        )
        || !is_unquoted_word(&tokens[maxdop_index].token, "MAXDOP")
    {
        return false;
    }

    matches!(tokens.get(value_index).map(|token| &token.token),
        Some(Token::Number(value, false)) if value.parse::<u16>().is_ok_and(|n| n <= 32_767))
}

fn contains_maxdop_hint_name(
    tokens: &[TokenWithSpan],
    option_index: usize,
    content_end: usize,
) -> bool {
    let Some(outer_close) = content_end.checked_sub(1) else {
        return false;
    };
    let Some(first_hint_index) = option_index.checked_add(2) else {
        return false;
    };

    let mut nested_depth = 0usize;
    let mut expecting_hint_name = true;
    for token in tokens.iter().take(outer_close).skip(first_hint_index) {
        match &token.token {
            Token::LParen => nested_depth += 1,
            Token::RParen if nested_depth > 0 => nested_depth -= 1,
            Token::Comma if nested_depth == 0 => expecting_hint_name = true,
            _ if nested_depth == 0 && expecting_hint_name => {
                if is_unquoted_word(&token.token, "MAXDOP") {
                    return true;
                }
                expecting_hint_name = false;
            }
            _ => {}
        }
    }
    false
}

fn parenthesized_suffix_ends_at(
    tokens: &[TokenWithSpan],
    opening_index: usize,
    content_end: usize,
) -> bool {
    let mut depth = 0usize;
    for (index, token) in tokens
        .iter()
        .enumerate()
        .take(content_end)
        .skip(opening_index)
    {
        match &token.token {
            Token::LParen => depth += 1,
            Token::RParen if depth > 0 => {
                depth -= 1;
                if depth == 0 {
                    return index + 1 == content_end;
                }
            }
            Token::RParen => return false,
            _ => {}
        }
    }
    false
}

fn is_word(token: &Token, keyword: Keyword, value: &str) -> bool {
    matches!(token, Token::Word(word)
        if word.quote_style.is_none() && word.keyword == keyword && word.value.eq_ignore_ascii_case(value))
}

fn is_unquoted_word(token: &Token, value: &str) -> bool {
    matches!(token, Token::Word(word)
        if word.quote_style.is_none() && word.value.eq_ignore_ascii_case(value))
}
