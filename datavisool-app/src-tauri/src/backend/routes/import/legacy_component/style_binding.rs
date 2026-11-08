use biome_js_syntax::{
    AnyJsExpression, JsAssignmentExpression, JsCallExpression, JsComputedMemberExpression, JsIdentifierAssignment, JsIdentifierBinding,
    JsIdentifierExpression, JsInstanceofExpression, JsReferenceIdentifier, JsStaticMemberAssignment, JsStaticMemberExpression,
    JsStringLiteralExpression, JsSyntaxNode, JsThisExpression, JsUnaryExpression, JsVariableDeclaration, JsVariableDeclarator,
};
use biome_rowan::{AstNode, AstSeparatedList};
use std::collections::{HashMap, HashSet};

pub(super) struct StyleElementBindingIndex {
    proven: HashSet<String>,
}

fn name(node: &impl AstNode<Language = biome_js_syntax::JsLanguage>) -> String {
    node.syntax().text_trimmed().to_string()
}

impl StyleElementBindingIndex {
    pub(super) fn new(syntax: &JsSyntaxNode) -> Self {
        let mut binding_counts: HashMap<String, usize> = HashMap::new();
        let mut binding_names: HashMap<String, JsIdentifierBinding> = HashMap::new();
        let mut references: HashMap<String, Vec<JsReferenceIdentifier>> = HashMap::new();
        let mut assignments = HashSet::new();
        let mut declarations: HashMap<String, Vec<JsVariableDeclarator>> = HashMap::new();
        let mut document_references = Vec::new();

        for node in syntax.descendants() {
            if let Some(binding) = JsIdentifierBinding::cast(node.clone()) {
                let binding_name = name(&binding);
                *binding_counts.entry(binding_name.clone()).or_default() += 1;
                binding_names.entry(binding_name).or_insert(binding);
            }
            if let Some(reference) = JsReferenceIdentifier::cast(node.clone()) {
                let reference_name = name(&reference);
                if reference_name == "document" {
                    document_references.push(reference.clone());
                }
                references.entry(reference_name).or_default().push(reference);
            }
            if let Some(assignment) = JsIdentifierAssignment::cast(node.clone()) {
                assignments.insert(name(&assignment));
            }
            if let Some(declarator) = JsVariableDeclarator::cast(node) {
                let Some(id) = declarator.id().ok() else { continue };
                let Some(binding) = id
                    .as_any_js_binding()
                    .and_then(|binding| binding.as_js_identifier_binding())
                    .cloned()
                else {
                    continue;
                };
                declarations.entry(name(&binding)).or_default().push(declarator);
            }
        }

        let document_shadowed = binding_counts.get("document").copied().unwrap_or_default() > 0 || assignments.contains("document");
        let global_object_unsafe = global_object_access_is_unsafe(syntax, &assignments);
        let mut style_aliases = HashSet::new();
        for (alias, candidates) in &declarations {
            if candidates.len() != 1 || binding_counts.get(alias).copied().unwrap_or_default() != 1 {
                continue;
            }
            let Some(binding) = binding_names.get(alias) else { continue };
            let Some(declarator) = candidates.first() else { continue };
            let Some(declarator_binding) = declarator.id().ok().and_then(|id| {
                id.as_any_js_binding()
                    .and_then(|binding| binding.as_js_identifier_binding())
                    .cloned()
            }) else {
                continue;
            };
            if binding.syntax() != declarator_binding.syntax() {
                continue;
            }
            let Some(declaration) = declarator.syntax().ancestors().find_map(JsVariableDeclaration::cast) else {
                continue;
            };
            if !declaration.kind().ok().is_some_and(|token| token.text_trimmed() == "const") {
                continue;
            }
            if is_style_initializer(declarator) {
                style_aliases.insert(alias.clone());
            }
        }

        let document_safe = !document_shadowed
            && !global_object_unsafe
            && document_references
                .iter()
                .all(|reference| document_reference_is_safe(reference, &style_aliases));
        let mut proven = HashSet::new();
        if document_safe {
            for alias in &style_aliases {
                if assignments.contains(alias) {
                    continue;
                }
                let mut writes = 0usize;
                let mut safe = true;
                for reference in references.get(alias).into_iter().flatten() {
                    if reference_is_safe_style_use(reference, &style_aliases) {
                        writes += 1;
                    } else {
                        safe = false;
                        break;
                    }
                }
                if safe && writes > 0 {
                    proven.insert(alias.clone());
                }
            }
        }
        Self { proven }
    }

    pub(super) fn is_proven(&self, alias: &str) -> bool {
        self.proven.contains(alias)
    }
}

fn is_style_initializer(declarator: &JsVariableDeclarator) -> bool {
    let Some(initializer) = declarator.initializer() else { return false };
    let Ok(expression) = initializer.expression() else { return false };
    let Some(call) = expression.as_js_call_expression() else { return false };
    let Ok(callee) = call.callee() else { return false };
    let Some(factory) = callee.as_js_static_member_expression() else {
        return false;
    };
    if !factory.member().ok().is_some_and(|method| name(&method) == "createElement")
        || !factory
            .object()
            .ok()
            .and_then(|object| object.as_js_identifier_expression().cloned())
            .is_some_and(|object| name(&object) == "document")
    {
        return false;
    }
    let Some(arguments) = call.arguments().ok() else { return false };
    let args: Vec<_> = arguments.args().iter().filter_map(Result::ok).collect();
    if args.len() != 1 {
        return false;
    }
    let Some(tag) = args[0].as_any_js_expression() else { return false };
    let Some(tag) = JsStringLiteralExpression::cast(tag.syntax().clone()) else {
        return false;
    };
    tag.value_token()
        .ok()
        .is_some_and(|token| matches!(token.text_trimmed(), "'style'" | "\"style\""))
}

fn document_reference_is_safe(reference: &JsReferenceIdentifier, style_aliases: &HashSet<String>) -> bool {
    let Some(identifier) = reference.syntax().parent().and_then(JsIdentifierExpression::cast) else {
        return false;
    };
    let Some(member) = identifier.syntax().parent().and_then(JsStaticMemberExpression::cast) else {
        return false;
    };
    if member.object().ok().is_none_or(|object| object.syntax() != identifier.syntax()) {
        return false;
    }
    let Ok(property) = member.member() else { return false };
    let property = name(&property);
    if property == "head" {
        let Some(append) = member.syntax().parent().and_then(JsStaticMemberExpression::cast) else {
            return false;
        };
        if append.object().ok().is_none_or(|object| object.syntax() != member.syntax())
            || !append
                .member()
                .ok()
                .is_some_and(|method| matches!(name(&method).as_str(), "appendChild" | "append"))
        {
            return false;
        }
        let Some(call) = append.syntax().parent().and_then(JsCallExpression::cast) else {
            return false;
        };
        if call.callee().ok().is_none_or(|callee| callee.syntax() != append.syntax()) {
            return false;
        }
        let Some(arguments) = call.arguments().ok() else { return false };
        let args: Vec<_> = arguments.args().iter().filter_map(Result::ok).collect();
        let method = append.member().ok().map(|method| name(&method)).unwrap_or_default();
        return static_style_arguments(&args, &method, None, style_aliases);
    }
    if !matches!(
        property.as_str(),
        "createElement" | "querySelector" | "querySelectorAll" | "getElementById" | "getElementsByClassName" | "getElementsByTagName"
    ) {
        return false;
    }
    let Some(call) = member.syntax().parent().and_then(JsCallExpression::cast) else {
        return false;
    };
    if call.callee().ok().is_none_or(|callee| callee.syntax() != member.syntax()) {
        return false;
    }
    if property == "createElement" {
        let Some(arguments) = call.arguments().ok() else { return false };
        let args: Vec<_> = arguments.args().iter().filter_map(Result::ok).collect();
        if args.len() != 1
            || args[0]
                .as_any_js_expression()
                .and_then(|arg| JsStringLiteralExpression::cast(arg.syntax().clone()))
                .is_none()
        {
            return false;
        }
    }
    for ancestor in call.syntax().ancestors().skip(1) {
        if let Some(member) = JsStaticMemberExpression::cast(ancestor.clone()) {
            let method = member.member().ok().map(|member| name(&member)).unwrap_or_default();
            let constructor_prototype = method == "prototype"
                && member
                    .object()
                    .ok()
                    .and_then(|object| object.as_js_static_member_expression().cloned())
                    .is_some_and(|object| object.member().ok().is_some_and(|property| name(&property) == "constructor"));
            if constructor_prototype || matches!(method.as_str(), "ownerDocument" | "head" | "createElement" | "appendChild" | "append" | "__proto__")
            {
                return false;
            }
        }
        if biome_js_syntax::JsComputedMemberExpression::can_cast(ancestor.kind()) {
            return false;
        }
        if ancestor
            .kind()
            .to_string()
            .is_some_and(|kind| matches!(kind, "JS_VARIABLE_DECLARATOR" | "JS_EXPRESSION_STATEMENT" | "JS_RETURN_STATEMENT"))
        {
            break;
        }
    }
    true
}

fn reference_is_safe_style_use(reference: &JsReferenceIdentifier, aliases: &HashSet<String>) -> bool {
    let Some(identifier) = reference.syntax().parent().and_then(JsIdentifierExpression::cast) else {
        return false;
    };
    if let Some(member) = identifier.syntax().parent().and_then(JsStaticMemberAssignment::cast) {
        if member.object().ok().is_none_or(|object| object.syntax() != identifier.syntax()) {
            return false;
        }
        if !member
            .member()
            .ok()
            .is_some_and(|property| matches!(name(&property).as_str(), "textContent" | "innerHTML" | "innerText"))
        {
            return false;
        }
        let Some(assignment) = member.syntax().parent().and_then(JsAssignmentExpression::cast) else {
            return false;
        };
        return assignment
            .left()
            .ok()
            .and_then(|left| left.as_any_js_assignment().cloned())
            .is_some_and(|left| left.syntax() == member.syntax())
            && assignment
                .operator_token()
                .ok()
                .is_some_and(|token| token.text_trimmed() == "=");
    }

    let Some(call) = identifier.syntax().ancestors().find_map(JsCallExpression::cast) else {
        return false;
    };
    let Some(callee) = call
        .callee()
        .ok()
        .and_then(|callee| callee.as_js_static_member_expression().cloned())
    else {
        return false;
    };
    if !callee
        .member()
        .ok()
        .is_some_and(|method| matches!(name(&method).as_str(), "appendChild" | "append"))
    {
        return false;
    }
    let Some(head) = callee
        .object()
        .ok()
        .and_then(|object| object.as_js_static_member_expression().cloned())
    else {
        return false;
    };
    if !head.member().ok().is_some_and(|property| name(&property) == "head")
        || !head
            .object()
            .ok()
            .and_then(|object| object.as_js_identifier_expression().cloned())
            .is_some_and(|object| name(&object) == "document")
    {
        return false;
    }
    let Some(arguments) = call.arguments().ok() else { return false };
    let args: Vec<_> = arguments.args().iter().filter_map(Result::ok).collect();
    let method = callee.member().ok().map(|method| name(&method)).unwrap_or_default();
    static_style_arguments(&args, &method, Some(identifier.syntax()), aliases)
}

fn static_style_arguments(
    args: &[biome_js_syntax::AnyJsCallArgument],
    method: &str,
    expected_argument: Option<&JsSyntaxNode>,
    aliases: &HashSet<String>,
) -> bool {
    if args.is_empty() || (method == "appendChild" && args.len() != 1) {
        return false;
    }
    let mut found_expected = expected_argument.is_none();
    for argument in args {
        let Some(expression) = argument.as_any_js_expression() else {
            return false;
        };
        let Some(identifier) = expression.as_js_identifier_expression() else {
            return false;
        };
        if !aliases.contains(&name(identifier)) {
            return false;
        }
        if expected_argument.is_some_and(|expected| expected == expression.syntax()) {
            found_expected = true
        }
    }
    found_expected
}

fn global_root_cached(expression: &AnyJsExpression, cache: &mut HashMap<JsSyntaxNode, Option<String>>) -> Option<String> {
    const GLOBAL_NAMES: &[&str] = &[
        "window",
        "globalThis",
        "self",
        "parent",
        "top",
        "Document",
        "HTMLDocument",
        "Node",
        "Element",
        "HTMLElement",
        "HTMLHeadElement",
        "DocumentFragment",
        "ShadowRoot",
        "Object",
        "Reflect",
    ];
    let mut chain = Vec::new();
    let mut current = expression.syntax().clone();
    let root = loop {
        if let Some(cached) = cache.get(&current) {
            break cached.clone();
        }
        if JsThisExpression::cast(current.clone()).is_some() {
            cache.insert(current.clone(), Some("this".to_string()));
            break Some("this".to_string());
        }
        if let Some(identifier) = JsIdentifierExpression::cast(current.clone()) {
            let identifier_name = name(&identifier);
            let global = is_global_root_name(&identifier_name).then_some(identifier_name);
            cache.insert(current.clone(), global.clone());
            break global;
        }
        if let Some(member) = JsStaticMemberExpression::cast(current.clone()) {
            chain.push(current.clone());
            let Some(object) = member.object().ok() else { break None };
            current = object.syntax().clone();
            continue;
        }
        if let Some(member) = JsComputedMemberExpression::cast(current.clone()) {
            chain.push(current.clone());
            let Some(object) = member.object().ok() else { break None };
            current = object.syntax().clone();
            continue;
        }
        cache.insert(current.clone(), None);
        break None;
    };
    for node in chain.into_iter().rev() {
        cache.insert(node, root.clone());
    }
    root
}

fn is_dom_constructor_name(name: &str) -> bool {
    matches!(
        name,
        "Document" | "HTMLDocument" | "Node" | "Element" | "HTMLElement" | "HTMLHeadElement" | "DocumentFragment" | "ShadowRoot"
    ) || (name.starts_with("HTML") && name.ends_with("Element") && name.len() > "HTMLElement".len())
}

fn is_global_root_name(name: &str) -> bool {
    matches!(name, "window" | "globalThis" | "self" | "parent" | "top" | "Object" | "Reflect") || is_dom_constructor_name(name)
}

fn unsafe_global_property(root: &str, property: &str) -> bool {
    const GLOBAL_OBJECTS: &[&str] = &["window", "globalThis", "self", "parent", "top"];
    if matches!(property, "document" | "parent" | "top" | "frames" | "self" | "window" | "globalThis") {
        return true;
    }
    if is_dom_constructor_name(root) && property == "prototype" {
        return true;
    }
    if matches!(root, "Object" | "Reflect") && matches!(property, "getPrototypeOf" | "setPrototypeOf") {
        return true;
    }
    GLOBAL_OBJECTS.contains(&root) && (is_dom_constructor_name(property) || matches!(property, "Object" | "Reflect"))
}

fn is_harmless_typeof(node: &JsSyntaxNode) -> bool {
    let Some(unary) = node.parent().and_then(JsUnaryExpression::cast) else {
        return false;
    };
    unary
        .operator_token()
        .ok()
        .is_some_and(|operator| operator.text_trimmed() == "typeof")
        && unary.argument().ok().is_some_and(|argument| argument.syntax() == node)
}

fn is_dom_instanceof_type(node: &JsSyntaxNode) -> bool {
    const GLOBAL_OBJECTS: &[&str] = &["window", "globalThis", "self", "parent", "top"];

    let Some(instanceof) = node.parent().and_then(JsInstanceofExpression::cast) else {
        return false;
    };
    let Some(right) = instanceof.right().ok() else {
        return false;
    };
    if right.syntax() != node {
        return false;
    }

    if let Some(identifier) = right.as_js_identifier_expression() {
        return is_dom_constructor_name(&name(identifier));
    }

    let Some(member) = right.as_js_static_member_expression() else {
        return false;
    };
    let Some(object) = member
        .object()
        .ok()
        .and_then(|object| object.as_js_identifier_expression().cloned())
    else {
        return false;
    };
    let global_object = name(&object);
    let constructor = member.member().ok().map(|member| name(&member)).unwrap_or_default();
    GLOBAL_OBJECTS.contains(&global_object.as_str()) && is_dom_constructor_name(&constructor)
}

fn global_object_access_is_unsafe(syntax: &JsSyntaxNode, assignments: &HashSet<String>) -> bool {
    const GLOBAL_NAMES: &[&str] = &[
        "window",
        "globalThis",
        "self",
        "parent",
        "top",
        "Document",
        "HTMLDocument",
        "Node",
        "Element",
        "HTMLElement",
        "HTMLHeadElement",
        "DocumentFragment",
        "ShadowRoot",
        "Object",
        "Reflect",
    ];
    if GLOBAL_NAMES.iter().any(|name| assignments.contains(*name)) {
        return true;
    }
    let mut global_root_cache: HashMap<JsSyntaxNode, Option<String>> = HashMap::new();
    for node in syntax.descendants() {
        if let Some(member) = JsStaticMemberExpression::cast(node.clone()) {
            let property_name = member.member().ok().map(|property| name(&property)).unwrap_or_default();
            let constructor_prototype = property_name == "prototype"
                && member
                    .object()
                    .ok()
                    .and_then(|object| object.as_js_static_member_expression().cloned())
                    .is_some_and(|object| object.member().ok().is_some_and(|property| name(&property) == "constructor"));
            if property_name == "__proto__" || constructor_prototype {
                return true;
            }
            if is_dom_instanceof_type(member.syntax()) {
                continue;
            }
            if let (Some(root), Some(property)) = (
                member
                    .object()
                    .ok()
                    .and_then(|object| global_root_cached(&object, &mut global_root_cache)),
                member.member().ok(),
            ) {
                if unsafe_global_property(&root, &name(&property)) {
                    return true;
                }
            }
        }
        if let Some(member) = JsComputedMemberExpression::cast(node.clone()) {
            if member
                .object()
                .ok()
                .is_some_and(|object| global_root_cached(&object, &mut global_root_cache).is_some())
            {
                return true;
            }
        }
        if let Some(reference) = JsReferenceIdentifier::cast(node.clone()) {
            let root = name(&reference);
            if !is_global_root_name(&root) {
                continue;
            }
            let Some(identifier) = reference.syntax().parent().and_then(JsIdentifierExpression::cast) else {
                return true;
            };
            if is_harmless_typeof(identifier.syntax()) {
                continue;
            }
            if is_dom_instanceof_type(identifier.syntax()) {
                continue;
            }
            let Some(member) = identifier.syntax().parent().and_then(JsStaticMemberExpression::cast) else {
                return true;
            };
            if is_dom_instanceof_type(member.syntax()) {
                continue;
            }
            if member.object().ok().is_none_or(|object| object.syntax() != identifier.syntax())
                || member
                    .member()
                    .ok()
                    .is_none_or(|property| unsafe_global_property(&root, &name(&property)))
            {
                return true;
            }
        }
        if let Some(this_expression) = JsThisExpression::cast(node) {
            let Some(member) = this_expression.syntax().parent().and_then(JsStaticMemberExpression::cast) else {
                return true;
            };
            if is_dom_instanceof_type(member.syntax()) {
                continue;
            }
            if member
                .object()
                .ok()
                .is_none_or(|object| object.syntax() != this_expression.syntax())
                || member
                    .member()
                    .ok()
                    .is_none_or(|property| unsafe_global_property("this", &name(&property)))
            {
                return true;
            }
        }
    }
    false
}
