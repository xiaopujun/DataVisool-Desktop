/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 导入时静态转换已识别的 LCP 组件接口，不执行组件代码。
//! TODO(legacy-compat): 停止支持 LCP 项目包后，移除此转换器及导入调用和相关测试。

use biome_js_parser::{parse_module, parse_script, JsParserOptions};
use biome_js_syntax::{
    AnyJsExpression, AnyJsParameter, JsArrowFunctionExpression, JsAssignmentExpression, JsCallExpression, JsComputedMemberExpression,
    JsFunctionExpression, JsIdentifierAssignment, JsIdentifierBinding, JsIdentifierExpression, JsName, JsNewExpression,
    JsPropertyObjectMember, JsReferenceIdentifier,
    JsStaticMemberAssignment, JsStaticMemberExpression, JsStringLiteralExpression, JsSyntaxNode, JsTemplateChunkElement, JsTemplateExpression,
    JsVariableDeclaration, JsVariableDeclarator,
};
use biome_rowan::{AstNode, AstNodeList, AstSeparatedList};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

mod bootstrap;
mod mapping;
mod package_scan;
mod source_safety;
mod style_binding;
mod styles;

use style_binding::StyleElementBindingIndex;

const OLD_REGISTRATION: &str = "__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__";
const OLD_BASE_MAP: &str = "__LCP_REMOTE_COMPONENT_BASE_URL_MAP__";
const OLD_GLOBALS: &[&str] = &["LightChaserInterface", "LightChaserUtil", "LightChaserUI", "LCPSDK", "LCPTypes", "LCPUI"];
const MAX_PACKAGE_CONVERT_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug)]
struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

fn decode_unicode(input: &str) -> String {
    static ESCAPE: OnceLock<regex::Regex> = OnceLock::new();
    let re = ESCAPE.get_or_init(|| regex::Regex::new(r"\\u(?:\{([0-9a-fA-F]{1,6})\}|([0-9a-fA-F]{4}))").unwrap());
    re.replace_all(input, |captures: &regex::Captures<'_>| {
        captures
            .get(1)
            .or_else(|| captures.get(2))
            .and_then(|m| u32::from_str_radix(m.as_str(), 16).ok())
            .and_then(char::from_u32)
            .map(|c| c.to_string())
            .unwrap_or_else(|| captures[0].to_string())
    })
    .into_owned()
}

fn text(node: &impl AstNode<Language = biome_js_syntax::JsLanguage>) -> String {
    decode_unicode(&node.syntax().text_trimmed().to_string())
}

fn range(node: &impl AstNode<Language = biome_js_syntax::JsLanguage>) -> (usize, usize) {
    let range = node.syntax().text_trimmed_range();
    (u32::from(range.start()) as usize, u32::from(range.end()) as usize)
}

// A static window/globalThis property is equivalent to the reviewed global identifier.
fn legacy_argument(expression: &AnyJsExpression) -> Option<(String, (usize, usize), Option<(usize, usize)>)> {
    if let Some(id) = expression.as_js_identifier_expression() {
        let name = text(id);
        return OLD_GLOBALS.contains(&name.as_str()).then(|| (name, range(id), None));
    }
    if let Some(member) = expression.as_js_static_member_expression() {
        if !matches!(text(&member.object().ok()?).as_str(), "window" | "globalThis") {
            return None;
        }
        let name_node = member.member().ok()?;
        let name = text(&name_node);
        return OLD_GLOBALS
            .contains(&name.as_str())
            .then(|| (name, range(member), Some(range(&name_node))));
    }
    if let Some(member) = expression.as_js_computed_member_expression() {
        if !matches!(text(&member.object().ok()?).as_str(), "window" | "globalThis") {
            return None;
        }
        let prop = member.member().ok()?;
        let literal = JsStringLiteralExpression::cast(prop.syntax().clone())?;
        let raw = literal.syntax().text_trimmed().to_string();
        if raw.len() < 2 || raw.contains('\\') {
            return None;
        }
        let name = raw[1..raw.len() - 1].to_string();
        return OLD_GLOBALS
            .contains(&name.as_str())
            .then(|| (name, range(member), Some(range(member))));
    }
    None
}

fn function_callee(mut expression: AnyJsExpression) -> Option<JsFunctionExpression> {
    loop {
        if let Some(function) = expression.as_js_function_expression() {
            return Some(function.clone());
        }
        expression = expression.as_js_parenthesized_expression()?.expression().ok()?;
    }
}

fn react_function_callee(mut expression: AnyJsExpression) -> Option<JsSyntaxNode> {
    loop {
        if let Some(function) = expression.as_js_function_expression() { return Some(function.syntax().clone()) }
        if let Some(function) = expression.as_js_arrow_function_expression() { return Some(function.syntax().clone()) }
        expression = expression.as_js_parenthesized_expression()?.expression().ok()?;
    }
}

fn react_parameter_index(function: &JsSyntaxNode, alias: &str) -> Option<usize> {
    let parameter_name = |parameter: &AnyJsParameter| {
        parameter.as_any_js_formal_parameter()
            .and_then(|p| p.as_js_formal_parameter())
            .and_then(|p| p.binding().ok())
            .and_then(|binding| binding.as_any_js_binding().and_then(|b| b.as_js_identifier_binding()).cloned())
            .map(|binding| text(&binding))
    };
    if let Some(function) = JsFunctionExpression::cast(function.clone()) {
        let parameters = function.parameters().ok()?;
        return parameters.items().iter().enumerate().find_map(|(index, parameter)| {
            parameter.as_ref().ok().and_then(parameter_name).filter(|name| name == alias).map(|_| index)
        });
    }
    let function = JsArrowFunctionExpression::cast(function.clone())?;
    let parameters = function.parameters().ok()?;
    if let Some(binding) = parameters.as_any_js_binding() {
        return binding.as_js_identifier_binding().map(|binding| text(binding) == alias).filter(|matches| *matches).map(|_| 0);
    }
    parameters.as_js_parameters()?.items().iter().enumerate().find_map(|(index, parameter)| {
        parameter.as_ref().ok().and_then(parameter_name).filter(|name| name == alias).map(|_| index)
    })
}

fn decode_js_style_text(raw: &str, template: bool) -> Option<String> {
    let mut chars = raw.chars().peekable();
    let mut out = String::with_capacity(raw.len());
    while let Some(ch) = chars.next() {
        if ch != '\\' { out.push(ch); continue }
        let escaped = chars.next()?;
        match escaped {
            '\\' | '\'' | '"' | '`' => out.push(escaped),
            '$' if template => out.push('$'),
            'n' => out.push('\n'), 'r' => out.push('\r'), 't' => out.push('\t'),
            'b' => out.push('\u{0008}'), 'f' => out.push('\u{000c}'), 'v' => out.push('\u{000b}'),
            '\n' => {},
            '\r' => { if chars.peek() == Some(&'\n') { chars.next(); } },
            'x' => {
                let hex: String = chars.by_ref().take(2).collect();
                if hex.len() != 2 { return None }
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            'u' => {
                let value = if chars.peek() == Some(&'{') {
                    chars.next(); let mut hex = String::new();
                    loop { let c = chars.next()?; if c == '}' { break } hex.push(c); }
                    if hex.is_empty() || hex.len() > 6 { return None }
                    u32::from_str_radix(&hex, 16).ok()?
                } else {
                    let hex: String = chars.by_ref().take(4).collect();
                    if hex.len() != 4 { return None }
                    u32::from_str_radix(&hex, 16).ok()?
                };
                out.push(char::from_u32(value)?);
            }
            other if template || !other.is_ascii_digit() => out.push(other),
            _ => return None,
        }
    }
    Some(out)
}

fn has_js_template_expression(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index + 1 < bytes.len() {
        if bytes[index] == b'$' && bytes[index + 1] == b'{' {
            let mut slashes = 0;
            let mut previous = index;
            while previous > 0 && bytes[previous - 1] == b'\\' { slashes += 1; previous -= 1; }
            if slashes % 2 == 0 { return true; }
            index += 2;
        } else {
            index += 1;
        }
    }
    false
}

fn proven_style_element_binding(syntax: &JsSyntaxNode, alias: &str) -> bool {
    StyleElementBindingIndex::new(syntax).is_proven(alias)
}

fn static_inline_css_context(expression: &JsSyntaxNode, bindings: &StyleElementBindingIndex) -> bool {
    let mut ancestor = expression.parent();
    while let Some(node) = ancestor {
        if let Some(call) = JsCallExpression::cast(node.clone()) {
            if let (Ok(args), Ok(callee)) = (call.arguments(), call.callee()) {
                let args: Vec<_> = args.args().iter().filter_map(Result::ok).collect();
                let index = args.iter().position(|arg| arg.as_any_js_expression().is_some_and(|value| value.syntax() == expression));
                if let (Some(index), Some(member)) = (index, callee.as_js_static_member_expression()) {
                    if let Ok(method) = member.member() {
                        let factory_object = member.object().ok().map(|object| text(&object));
                        let react_factory = factory_object.as_deref() == Some("React")
                            || factory_object.as_deref().is_some_and(|alias| {
                                call.syntax().ancestors().any(|node| {
                                    let Some(param_index) = react_parameter_index(&node, alias) else { return false };
                                    node.ancestors().filter_map(JsCallExpression::cast).any(|outer_call| {
                                        let wraps_function = outer_call.callee().ok().and_then(react_function_callee)
                                            .is_some_and(|callee| callee == node);
                                        let passes_react = outer_call.arguments().ok()
                                            .and_then(|arguments| arguments.args().iter().nth(param_index).and_then(Result::ok))
                                            .and_then(|argument| argument.as_any_js_expression().cloned())
                                            .and_then(|argument| argument.as_js_identifier_expression().cloned())
                                            .is_some_and(|argument| text(&argument) == "React");
                                        wraps_function && passes_react
                                    })
                                })
                            });
                        if text(&method) == "createElement" && react_factory && index >= 2 {
                            if let Some(tag) = args.first().and_then(|arg| arg.as_any_js_expression()) {
                                if let Some(tag) = JsStringLiteralExpression::cast(tag.syntax().clone()) {
                                    if tag.value_token().ok().is_some_and(|value| matches!(value.text_trimmed(), "'style'" | "\"style\"")) {
                                        return true;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(assignment) = JsAssignmentExpression::cast(node.clone()) {
            let right_matches = assignment.right().ok().is_some_and(|right| right.syntax() == expression);
            let plain = assignment.operator_token().ok().is_some_and(|token| token.text_trimmed() == "=");
            if right_matches && plain {
                if let Some(left) = assignment.left().ok().and_then(|left| left.as_any_js_assignment().cloned()) {
                    if let Some(member) = left.as_js_static_member_assignment() {
                        if member.member().ok().is_some_and(|name| text(&name) == "cssText") {
                            if let Some(style) = member.object().ok().and_then(|object| object.as_js_static_member_expression().cloned()) {
                                if style.member().ok().is_some_and(|name| text(&name) == "style") { return true }
                            }
                        }
                        if member.member().ok().is_some_and(|name| matches!(text(&name).as_str(), "textContent" | "innerHTML" | "innerText")) {
                            if let Some(object) = member.object().ok().and_then(|object| object.as_js_identifier_expression().cloned()) {
                                if bindings.is_proven(&text(&object)) { return true }
                            }
                        }
                    }
                }
            }
        }
        ancestor = node.parent();
    }
    false
}

fn style_edits(syntax: &JsSyntaxNode) -> Result<Vec<Edit>, String> {
    // Only direct, unescaped className literals have unambiguous semantics.
    let mut edits = Vec::new();
    let mut converted_static_templates = HashSet::new();
    let style_bindings = StyleElementBindingIndex::new(syntax);
    for template in syntax.descendants().filter_map(JsTemplateExpression::cast) {
        if !static_inline_css_context(template.syntax(), &style_bindings) { continue }
        let elements = template.elements();
        let mut css = String::new();
        let source = template.syntax().text_trimmed().to_string();
        let mut is_static = !has_js_template_expression(&source);
        for element in elements.iter() {
            let Some(chunk) = element.as_js_template_chunk_element() else { is_static = false; continue };
            let token = chunk.template_chunk_token().map_err(|_| "脚本中的 CSS 模板不完整")?;
            css.push_str(&decode_js_style_text(token.text_trimmed(), true).ok_or("脚本中的 CSS 模板转义无法解码")?);
        }
        if !is_static {
            if styles::contains_legacy_style_prefix(&css)? || styles::contains_legacy_style_prefix(&source)? {
                return Err("动态脚本样式模板无法安全转换".into());
            }
            continue;
        }
        let converted = styles::convert(&css)?;
        let (start, end) = range(&template);
        if converted != css {
            edits.push(Edit { start, end, replacement: serde_json::to_string(&converted).map_err(|_| "脚本中的 CSS 无法编码")? });
        }
        converted_static_templates.insert((start, end));
    }
    for literal in syntax.descendants().filter_map(JsStringLiteralExpression::cast) {
        let raw = literal.syntax().text_trimmed().to_string();
        if raw.len() < 2 {
            continue;
        }
        let value = &raw[1..raw.len() - 1];
        if static_inline_css_context(literal.syntax(), &style_bindings) {
            let css = decode_js_style_text(value, false).ok_or("脚本中的 CSS 字符串转义无法解码")?;
            let converted = styles::convert(&css)?;
            if converted != css {
                let (start, end) = range(&literal);
                edits.push(Edit { start, end, replacement: serde_json::to_string(&converted).map_err(|_| "脚本中的 CSS 无法编码")? });
            }
            continue;
        }
        let property = literal.syntax().parent().and_then(JsPropertyObjectMember::cast);
        let is_class_name = property.as_ref().is_some_and(|p| {
            p.name()
                .ok()
                .is_some_and(|name| text(&name).trim_matches(['\'', '"']) == "className")
                && p.value().ok().is_some_and(|v| v.syntax() == literal.syntax())
        });
        let decoded_value = if is_class_name {
            decode_js_style_text(value, false).ok_or("className 字符串转义无法解码")?
        } else {
            decode_unicode(value)
        };
        // URLs and asset filenames are data, not CSS selectors or class names.
        let lower = decoded_value.to_ascii_lowercase();
        let resource = lower.starts_with('/')
            || lower.starts_with("./")
            || lower.starts_with("../")
            || lower.starts_with("https:")
            || lower.starts_with("http:")
            || lower.starts_with("data:")
            || [".js", ".mjs", ".css", ".svg", ".png", ".jpg", ".jpeg", ".gif", ".webp", ".woff", ".woff2", ".ttf"]
                .iter()
                .any(|ext| lower.split(['?', '#']).next().unwrap_or("").ends_with(ext));
        if resource && !is_class_name {
            continue;
        }
        let has_class = styles::contains_known_class(&decoded_value);
        if decoded_value.contains(".lcp-") || decoded_value.contains("--lcp-") || decoded_value.contains(".lc-dialog") {
            return Err("脚本中的旧 UI 样式无法安全转换".into());
        }
        if !has_class {
            continue;
        }
        if !is_class_name {
            return Err("脚本中的旧 UI 类名无法安全转换".into());
        }
        let mapped = styles::map_class_names(&decoded_value);
        if mapped == decoded_value {
            return Err("脚本中的旧 UI 类名无法安全转换".into());
        }
        let (start, end) = range(&literal);
        edits.push(Edit { start, end, replacement: serde_json::to_string(&mapped).map_err(|_| "旧 UI 类名无法编码")? });
    }
    for template in syntax.descendants().filter_map(JsTemplateChunkElement::cast) {
        let handled_css = template.syntax().ancestors().find_map(JsTemplateExpression::cast)
            .is_some_and(|expr| converted_static_templates.contains(&range(&expr)));
        if handled_css { continue }
        let raw = template.syntax().text_trimmed().to_string();
        let value = decode_js_style_text(&raw, true).ok_or("脚本模板转义无法解码")?;
        if styles::contains_legacy_style_prefix(&value)? || styles::contains_known_class(&value) {
            return Err("脚本模板中的旧 UI 样式无法安全转换".into());
        }
    }

    Ok(edits)
}

/// 已知旧组件转换为当前全局入口；结构或接口不明确时返回错误。
pub(super) fn convert_entry(source: &str) -> Result<Option<String>, String> {
    let decoded = if source.contains("\\u") {
        decode_unicode(source)
    } else {
        source.to_string()
    };
    let has_old_api = OLD_GLOBALS
        .iter()
        .chain([OLD_REGISTRATION, OLD_BASE_MAP].iter())
        .any(|needle| source.contains(needle) || decoded.contains(needle));
    if !has_old_api {
        if !decoded.contains("lcp-") && !decoded.contains("lc-dialog")
            && !styles::contains_legacy_style_candidate(source)
        {
            return Ok(None);
        }
        if source.len() > 4 * 1024 * 1024 {
            return Err("组件超出旧 UI 样式转换大小限制".into());
        }
        let parser_source = source_safety::parser_source(source)?;
        let script = parse_script(&parser_source, JsParserOptions::default());
        let syntax = if !script.has_errors() {
            script.syntax()
        } else {
            let module = parse_module(&parser_source, JsParserOptions::default());
            if module.has_errors() {
                return Err("组件 JavaScript 无法解析".into());
            }
            module.syntax()
        };
        let mut edits = style_edits(&syntax)?;
        if edits.is_empty() {
            return Ok(None);
        }
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
        let mut converted = source.to_string();
        for edit in edits {
            converted.replace_range(edit.start..edit.end, &edit.replacement);
        }
        return Ok(Some(converted));
    }
    if let Some((prefix, entry)) = bootstrap::split(source)? {
        let converted = convert_entry(entry)?.ok_or("旧组件资源入口无法静态转换")?;
        return Ok(Some(prefix + &converted));
    }
    if source.len() > 4 * 1024 * 1024 {
        return Err("组件超出旧组件转换大小限制".into());
    }
    let parser_source = source_safety::parser_source(source)?;
    let script = parse_script(&parser_source, JsParserOptions::default());
    let syntax = if !script.has_errors() {
        script.syntax()
    } else {
        let module = parse_module(&parser_source, JsParserOptions::default());
        if module.has_errors() {
            return Err("组件 JavaScript 无法解析".into());
        }
        module.syntax()
    };
    // 被替换的名字必须指向真正的宿主全局，不能有任何同名局部绑定。
    for binding in syntax.descendants().filter_map(JsIdentifierBinding::cast) {
        let name = text(&binding);
        if OLD_GLOBALS.contains(&name.as_str())
            || matches!(name.as_str(), "DataVisoolSDK" | "DataVisoolTypes" | "DataVisoolUI" | "globalThis" | "window")
        {
            return Err(format!("组件中存在无法安全转换的同名绑定：{name}"));
        }
    }
    let declared_entry_call = syntax.descendants().filter_map(JsVariableDeclarator::cast).find_map(|decl| {
        let id = decl.id().ok()?;
        if text(&id) != OLD_REGISTRATION {
            return None;
        }
        let expr = decl.initializer()?.expression().ok()?;
        expr.as_js_call_expression().map(range)
    });
    let mut edits = Vec::<Edit>::new();
    let mut approved_refs = HashSet::<(usize, usize)>::new();
    let mut approved_names = HashSet::<(usize, usize)>::new();
    let mut approved_computed = HashSet::<(usize, usize)>::new();
    let mut wrappers = 0usize;
    let mut wrapper_body = None;
    let mut wrapper_call = None;
    for call in syntax.descendants().filter_map(JsCallExpression::cast) {
        let args = call.arguments().map_err(|_| "组件调用参数不完整")?;
        let mut old_arguments = Vec::new();
        for (index, arg) in args.args().iter().enumerate() {
            let arg = arg.map_err(|_| "组件调用参数不完整")?;
            let Some(expression) = arg.as_any_js_expression() else { continue };
            if let Some((name, arg_range, property_range)) = legacy_argument(expression) {
                old_arguments.push((index, name, arg_range, property_range));
            }
        }
        let function = call.callee().ok().and_then(function_callee);
        if old_arguments.is_empty() && declared_entry_call != Some(range(&call)) {
            let has_registration = function.as_ref().is_some_and(|f| {
                f.body().ok().is_some_and(|body| {
                    body.syntax()
                        .descendants()
                        .filter_map(JsName::cast)
                        .any(|name| text(&name) == OLD_REGISTRATION)
                })
            });
            if !has_registration {
                continue;
            }
        }
        let function = function.ok_or("旧组件接口未用于可识别的直接函数调用")?;
        for arg in args.args().iter() {
            let arg = arg.map_err(|_| "组件调用参数不完整")?;
            let expression = arg.as_any_js_expression().ok_or("旧组件入口使用了无法识别的依赖")?;
            if legacy_argument(expression).is_some() {
                continue;
            }
            let ident = expression
                .as_js_identifier_expression()
                .ok_or("旧组件入口使用了无法识别的依赖")?;
            let name = text(ident);
            if !matches!(name.as_str(), "React" | "ReactDOM") {
                return Err(format!("不支持的组件依赖：{name}"));
            }
        }
        let parameters = function
            .parameters()
            .map_err(|_| "旧组件入口参数不完整")?
            .items()
            .iter()
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "旧组件入口参数不完整")?;
        let body = function.body().map_err(|_| "旧组件入口缺少函数体")?;
        wrapper_body = Some(range(&body));
        wrapper_call = Some(range(&call));
        let mut aliases = HashMap::<String, String>::new();
        for (index, global, arg_range, property_range) in old_arguments {
            let target = mapping::target(&global).ok_or_else(|| format!("暂不支持旧组件接口：{global}"))?;
            let param = parameters
                .get(index)
                .and_then(|p| p.as_any_js_formal_parameter())
                .and_then(|p| p.as_js_formal_parameter())
                .ok_or("旧组件入口参数不是普通标识符")?;
            let binding = param.binding().map_err(|_| "旧组件入口参数不完整")?;
            let binding = binding
                .as_any_js_binding()
                .and_then(|b| b.as_js_identifier_binding())
                .ok_or("旧组件入口使用了解构参数")?;
            let alias = text(binding);
            if aliases.insert(alias.clone(), global.clone()).is_some() {
                return Err("旧组件入口存在重复接口参数".into());
            }
            let (start, end) = arg_range;
            if let Some(property) = property_range {
                if property == arg_range {
                    approved_computed.insert(arg_range);
                } else {
                    approved_names.insert(property);
                }
            } else {
                approved_refs.insert(arg_range);
            }
            edits.push(Edit { start, end, replacement: target.to_string() });
        }
        // Nested function parameters may shadow an outer alias; leave those scopes untouched.
        // Other kinds of bindings need lexical analysis and are rejected rather than guessed.
        let mut shadows: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
        for binding in body.syntax().descendants().filter_map(JsIdentifierBinding::cast) {
            let alias = text(&binding);
            if !aliases.contains_key(&alias) {
                continue;
            }
            let parent = binding.syntax().parent().ok_or("旧接口遮蔽参数不完整")?;
            let parent_kind = format!("{:?}", parent.kind());
            let is_param = matches!(parent_kind.as_str(), "JS_FORMAL_PARAMETER" | "JS_ARROW_FUNCTION_EXPRESSION");
            let is_var = if parent_kind == "JS_VARIABLE_DECLARATOR" {
                parent
                    .ancestors()
                    .find_map(JsVariableDeclaration::cast)
                    .and_then(|decl| decl.kind().ok())
                    .is_some_and(|token| token.text_trimmed() == "var")
            } else {
                false
            };
            if !is_param && !is_var {
                return Err(format!("旧组件接口参数被不支持的局部变量遮蔽：{alias}"));
            }
            let function = parent
                .ancestors()
                .find(|node| {
                    matches!(
                        format!("{:?}", node.kind()).as_str(),
                        "JS_FUNCTION_DECLARATION"
                            | "JS_FUNCTION_EXPRESSION"
                            | "JS_ARROW_FUNCTION_EXPRESSION"
                            | "JS_METHOD_CLASS_MEMBER"
                            | "JS_METHOD_OBJECT_MEMBER"
                            | "JS_CONSTRUCTOR_CLASS_MEMBER"
                            | "JS_GETTER_CLASS_MEMBER"
                            | "JS_SETTER_CLASS_MEMBER"
                            | "JS_GETTER_OBJECT_MEMBER"
                            | "JS_SETTER_OBJECT_MEMBER"
                    )
                })
                .ok_or("无法确定旧组件遮蔽参数的作用域")?;
            let scope = function.text_trimmed_range();
            let (start, end) = (u32::from(scope.start()) as usize, u32::from(scope.end()) as usize);
            let (body_start, body_end) = range(&body);
            if start < body_start || end > body_end {
                return Err(format!("旧组件接口参数被入口局部变量覆盖：{alias}"));
            }
            shadows.entry(alias).or_default().push((start, end));
        }
        for identifier in body.syntax().descendants().filter_map(JsIdentifierExpression::cast) {
            let alias = text(&identifier);
            let Some(global) = aliases.get(&alias) else { continue };
            let (offset, _) = range(&identifier);
            if shadows
                .get(&alias)
                .is_some_and(|scopes| scopes.iter().any(|(start, end)| offset >= *start && offset < *end))
            {
                continue;
            }
            let parent = identifier
                .syntax()
                .parent()
                .and_then(JsStaticMemberExpression::cast)
                .ok_or_else(|| format!("旧接口 {global} 存在无法静态分析的引用"))?;
            let object = parent.object().map_err(|_| "旧接口访问不完整")?;
            if object.syntax() != identifier.syntax() {
                return Err(format!("旧接口 {global} 存在无法静态分析的引用"));
            }
            let property = parent.member().map_err(|_| "旧接口成员不完整")?;
            let property = text(&property);
            if global == "LightChaserUtil" && property == "FetchUtil" {
                return Err("旧组件使用了 FetchUtil；共享 SDK 未提供对应网络工具，不支持自动转换，无法导入".into());
            }
            if !mapping::allowed(global, &property) {
                return Err(format!("旧组件使用了暂不支持的接口：{global}.{property}"));
            }
            if matches!(global.as_str(), "LightChaserUI" | "LCPUI") {
                if let Some(renamed) = mapping::ui_name(&property) {
                    let (start, end) = range(&parent.member().map_err(|_| "旧组件 UI 成员不完整")?);
                    edits.push(Edit { start, end, replacement: renamed.to_string() });
                }
            }
            if global == "LightChaserUtil" {
                // Reading a utility as an unused expression has no observable method semantics.
                if parent
                    .syntax()
                    .parent()
                    .is_some_and(|node| format!("{:?}", node.kind()) == "JS_EXPRESSION_STATEMENT")
                {
                    continue;
                }
                // Other utility references require a statically known method.
                let method = parent
                    .syntax()
                    .parent()
                    .and_then(JsStaticMemberExpression::cast)
                    .ok_or_else(|| format!("旧接口 {global}.{property} 的方法无法静态确认"))?;
                if method.object().map_err(|_| "旧接口方法不完整")?.syntax() != parent.syntax() {
                    return Err(format!("旧接口 {global}.{property} 的方法无法静态确认"));
                }
                let method_name = text(&method.member().map_err(|_| "旧接口方法不完整")?);
                if !mapping::util_method(&property, &method_name) {
                    return Err(format!("旧组件使用了暂不支持的接口：{global}.{property}.{method_name}"));
                }
                if property == "ObjectUtil" && method_name == "merge" {
                    let call = method
                        .syntax()
                        .parent()
                        .and_then(JsCallExpression::cast)
                        .ok_or("旧版 ObjectUtil.merge 只能以直接调用方式转换")?;
                    if call.callee().map_err(|_| "旧版 ObjectUtil.merge 调用不完整")?.syntax() != method.syntax() {
                        return Err("旧版 ObjectUtil.merge 只能以直接调用方式转换".into());
                    }
                    let (start, end) = range(&method);
                    edits.push(Edit { start, end, replacement: mapping::LEGACY_MERGE.to_string() });
                }
            }
        }
        wrappers += 1;
    }
    for assignment in syntax.descendants().filter_map(JsIdentifierAssignment::cast) {
        let name = text(&assignment);
        if OLD_GLOBALS.contains(&name.as_str()) || name == OLD_REGISTRATION || name == OLD_BASE_MAP {
            return Err(format!("旧接口或注册标记被直接赋值，无法安全转换：{name}"));
        }
    }
    for reference in syntax.descendants().filter_map(JsReferenceIdentifier::cast) {
        let name = text(&reference);
        if name == OLD_REGISTRATION || name == OLD_BASE_MAP {
            let (start, end) = range(&reference);
            edits.push(Edit {
                start,
                end,
                replacement: if name == OLD_REGISTRATION {
                    "__DATAVISOOL_REMOTE_COMPONENT_TEMP__".into()
                } else {
                    "__DATAVISOOL_REMOTE_COMPONENT_BASE_URL_MAP__".into()
                },
            });
        }
        if OLD_GLOBALS.contains(&name.as_str()) && !approved_refs.contains(&range(&reference)) {
            return Err(format!("旧接口 {name} 的使用方式无法静态转换"));
        }
        if (name == "eval" || name == "Function") && wrappers > 0 {
            let expression = reference.syntax().parent().and_then(JsIdentifierExpression::cast);
            let dynamic = expression
                .as_ref()
                .and_then(|node| node.syntax().parent())
                .is_some_and(|parent| {
                    if let Some(call) = JsCallExpression::cast(parent.clone()) {
                        return call
                            .callee()
                            .ok()
                            .is_some_and(|callee| Some(callee.syntax()) == expression.as_ref().map(|v| v.syntax()));
                    }
                    if let Some(call) = JsNewExpression::cast(parent) {
                        return call
                            .callee()
                            .ok()
                            .is_some_and(|callee| Some(callee.syntax()) == expression.as_ref().map(|v| v.syntax()));
                    }
                    false
                });
            if dynamic {
                return Err("旧组件包含动态代码，无法安全静态转换".into());
            }
        }
    }
    edits.extend(style_edits(&syntax)?);

    let mut registrations = 0usize;
    // Legacy Web and CLI bundles may register by declaring the top-level slot.
    for binding in syntax.descendants().filter_map(JsIdentifierBinding::cast) {
        if text(&binding) != OLD_REGISTRATION {
            continue;
        }
        let declarator = binding
            .syntax()
            .parent()
            .and_then(JsVariableDeclarator::cast)
            .ok_or("旧组件注册标记不是变量声明")?;
        let declaration = declarator
            .syntax()
            .ancestors()
            .find_map(JsVariableDeclaration::cast)
            .ok_or("无法确认旧组件注册变量的作用域")?;
        if !declaration.kind().ok().is_some_and(|token| token.text_trimmed() == "var") {
            return Err("旧组件注册变量不是全局 var 声明".into());
        }
        if declarator.syntax().ancestors().any(|node| {
            matches!(
                format!("{:?}", node.kind()).as_str(),
                "JS_FUNCTION_DECLARATION"
                    | "JS_FUNCTION_EXPRESSION"
                    | "JS_ARROW_FUNCTION_EXPRESSION"
                    | "JS_METHOD_CLASS_MEMBER"
                    | "JS_METHOD_OBJECT_MEMBER"
                    | "JS_CONSTRUCTOR_CLASS_MEMBER"
            )
        }) {
            return Err("旧组件注册变量不在全局作用域".into());
        }
        let initializer = declarator.initializer().ok_or("旧组件注册标记缺少入口")?;
        let expression = initializer.expression().map_err(|_| "旧组件入口不完整")?;
        if wrapper_call != Some(range(&expression)) {
            return Err("旧组件注册入口无法静态确认".into());
        }
        let (start, end) = range(&binding);
        edits.push(Edit { start, end, replacement: "__DATAVISOOL_REMOTE_COMPONENT_TEMP__".into() });
        registrations += 1;
    }
    for name in syntax.descendants().filter_map(JsName::cast) {
        let value = text(&name);
        if OLD_GLOBALS.contains(&value.as_str()) && !approved_names.contains(&range(&name)) {
            return Err(format!("旧接口 {value} 使用了无法静态转换的属性访问"));
        }
        if value != OLD_REGISTRATION && value != OLD_BASE_MAP {
            continue;
        }
        let parent = name.syntax().parent().ok_or("旧组件全局引用不完整")?;
        let (object, is_assignment) = if let Some(member) = JsStaticMemberExpression::cast(parent.clone()) {
            (member.object().map_err(|_| "旧组件全局引用不完整")?, false)
        } else if let Some(member) = JsStaticMemberAssignment::cast(parent) {
            (member.object().map_err(|_| "旧组件全局引用不完整")?, true)
        } else {
            return Err("旧组件标记不属于可识别的全局访问".into());
        };
        if !matches!(text(&object).as_str(), "globalThis" | "window") {
            return Err("旧组件标记未引用宿主全局对象".into());
        }
        if value == OLD_REGISTRATION && !is_assignment {
            return Err("旧组件注册标记不是赋值操作".into());
        }
        if value == OLD_REGISTRATION {
            let (start, end) = range(&name);
            if !wrapper_body.is_some_and(|(body_start, body_end)| start >= body_start && end <= body_end) {
                return Err("旧组件注册不在入口函数内".into());
            }
            let assignment = name
                .syntax()
                .parent()
                .and_then(|parent| parent.parent())
                .and_then(JsAssignmentExpression::cast)
                .ok_or("旧组件注册标记不是简单赋值")?;
            if assignment.operator_token().map_err(|_| "旧组件注册操作不完整")?.text_trimmed() != "=" {
                return Err("旧组件注册标记不是简单赋值".into());
            }
        }
        if value == OLD_REGISTRATION {
            registrations += 1;
        }
        let (start, end) = range(&name);
        edits.push(Edit {
            start,
            end,
            replacement: if value == OLD_REGISTRATION {
                "__DATAVISOOL_REMOTE_COMPONENT_TEMP__"
            } else {
                "__DATAVISOOL_REMOTE_COMPONENT_BASE_URL_MAP__"
            }
            .into(),
        });
    }
    // 不猜测计算属性或动态宿主全局的含义。
    if OLD_GLOBALS.iter().any(|name| source.contains(name) || decoded.contains(name)) {
        for member in syntax.descendants().filter_map(JsComputedMemberExpression::cast) {
            if member
                .object()
                .ok()
                .is_some_and(|object| matches!(text(&object).as_str(), "globalThis" | "window"))
            {
                if approved_computed.contains(&range(&member)) {
                    continue;
                }
                let literal = member.member().ok();
                let safe_literal = literal
                    .as_ref()
                    .is_some_and(|value| value.as_any_js_literal_expression().is_some());
                if !safe_literal {
                    return Err("旧组件使用动态全局属性，无法安全静态转换".into());
                }
                if literal
                    .as_ref()
                    .is_some_and(|value| OLD_GLOBALS.iter().any(|name| text(value).contains(name)))
                {
                    return Err("旧接口的全局属性访问无法静态转换".into());
                }
            }
        }
    }
    if edits.is_empty() {
        return Ok(None);
    }
    if (wrappers != 0 || registrations != 0) && (wrappers != 1 || registrations != 1) {
        return Err("旧组件入口或注册标记不唯一，无法安全静态转换".into());
    }
    edits.sort_by_key(|edit| std::cmp::Reverse(edit.start));
    let mut converted = source.to_string();
    for edit in edits {
        converted.replace_range(edit.start..edit.end, &edit.replacement);
    }
    let validated_source = source_safety::parser_source(&converted)?;
    let parsed = parse_script(&validated_source, JsParserOptions::default());
    if parsed.has_errors()
        && ((wrappers != 0 || registrations != 0) || parse_module(&validated_source, JsParserOptions::default()).has_errors())
    {
        return Err("转换后的组件不是可直接加载的普通脚本".into());
    }
    Ok(Some(converted))
}

/// 在暂存目录转换所有 JavaScript；不支持的旧代码会中止整个导入。
pub(super) fn convert_package(root: &Path, entry: &str) -> Result<bool, String> {
    fn visit(root: &Path, dir: &Path, entry: &Path, converted: &mut bool, js_converted: &mut bool) -> Result<(), String> {
        for item in std::fs::read_dir(dir).map_err(|e| format!("读取组件目录失败：{e}"))? {
            let item = item.map_err(|e| format!("读取组件文件失败：{e}"))?;
            let path = item.path();
            let kind = item.file_type().map_err(|e| format!("读取组件文件类型失败：{e}"))?;
            if kind.is_dir() {
                visit(root, &path, entry, converted, js_converted)?;
                continue;
            }
            if !kind.is_file() {
                return Err("组件包包含不支持的文件类型".into());
            }
            let is_js = path
                .extension()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.eq_ignore_ascii_case("js") || v.eq_ignore_ascii_case("mjs"));
            let is_css = path
                .extension()
                .and_then(|v| v.to_str())
                .is_some_and(|v| v.eq_ignore_ascii_case("css"));
            let size = std::fs::metadata(&path).map_err(|e| format!("读取组件文件元数据失败：{e}"))?.len();
            if is_css {
                if size > MAX_PACKAGE_CONVERT_BYTES {
                    match package_scan::scan_large_source(&path, package_scan::SourceKind::Css)? {
                        package_scan::ScanOutcome::Clean => continue,
                        package_scan::ScanOutcome::Candidate => return Err("组件样式超出旧 UI 样式转换大小限制".into()),
                        package_scan::ScanOutcome::Uncertain => return Err("超大组件样式含无法安全判断的转义，已拒绝导入".into()),
                    }
                }
                let bytes = read_bounded(&path, "读取组件样式失败")?;
                let source = std::str::from_utf8(&bytes).map_err(|_| "组件样式不是 UTF-8")?;
                let output = styles::convert(source)?;
                if output != source {
                    std::fs::write(&path, output).map_err(|e| format!("写入转换后的组件样式失败：{e}"))?;
                    *converted = true;
                }
                continue;
            }
            if !is_js {
                continue;
            }
            if size > MAX_PACKAGE_CONVERT_BYTES {
                match package_scan::scan_large_source(&path, package_scan::SourceKind::JavaScript)? {
                    package_scan::ScanOutcome::Clean => continue,
                    package_scan::ScanOutcome::Candidate => return Err(format!("{}：组件超出旧组件转换大小限制", path.display())),
                    package_scan::ScanOutcome::Uncertain => return Err(format!("{}：超大组件含无法安全判断的转义，已拒绝导入", path.display())),
                }
            }
            let bytes = read_bounded(&path, "读取组件脚本失败")?;
            let source = std::str::from_utf8(&bytes).map_err(|_| "组件脚本不是 UTF-8".to_string())?;
            let rel = path.strip_prefix(root).map_err(|_| "组件路径不合法")?;
            if let Some(output) = convert_entry(source).map_err(|e| format!("{}：{e}", rel.display()))? {
                let decoded = decode_unicode(source);
                let has_legacy_api = OLD_GLOBALS
                    .iter()
                    .chain([OLD_REGISTRATION, OLD_BASE_MAP].iter())
                    .any(|name| decoded.contains(name));
                if has_legacy_api && (rel != entry || *js_converted) {
                    return Err(format!("{}：旧组件代码不在唯一入口文件中", rel.display()));
                }
                std::fs::write(&path, output).map_err(|e| format!("写入转换后的组件失败：{e}"))?;
                *converted = true;
                if has_legacy_api {
                    *js_converted = true;
                }
            }
        }
        Ok(())
    }
    let mut converted = false;
    let mut js_converted = false;
    visit(root, root, Path::new(entry), &mut converted, &mut js_converted)?;
    Ok(converted)
}

fn read_bounded(path: &Path, context: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| format!("{context}：{e}"))?;
    let initial_capacity = file
        .metadata()
        .map_err(|e| format!("{context}：{e}"))?
        .len()
        .min(MAX_PACKAGE_CONVERT_BYTES) as usize;
    let mut bytes = Vec::with_capacity(initial_capacity);
    file.take(MAX_PACKAGE_CONVERT_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("{context}：{e}"))?;
    if bytes.len() as u64 > MAX_PACKAGE_CONVERT_BYTES {
        return Err("组件文件大小在扫描期间发生变化，已拒绝导入".into());
    }
    Ok(bytes)
}

#[cfg(test)]
#[path = "legacy_component/tests.rs"]
mod tests;
