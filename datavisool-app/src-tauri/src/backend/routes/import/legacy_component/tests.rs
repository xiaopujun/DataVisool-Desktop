use super::convert_entry;

const SIMPLE: &str = r#"(function(R, I, U){
        const C = class extends I.AbstractDesignerDefinition {};
        I.BaseInfo; U.ObjectUtil.merge({}, {}); R.createElement;
        globalThis.__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__ = C;
    })(React, LightChaserInterface, LightChaserUtil);"#;

#[test]
fn rewrites_interface_globals_and_registration_without_touching_literals() {
    let input = format!("const note = 'LightChaserInterface';\n{SIMPLE}");
    let result = convert_entry(&input).unwrap().unwrap();
    assert!(result.contains("const note = 'LightChaserInterface'"));
    assert!(result.contains("})(React, DataVisoolSDK, DataVisoolSDK)"));
    assert!(result.contains("globalThis.__DATAVISOOL_REMOTE_COMPONENT_TEMP__ = C"));
    assert!(convert_entry(&result).unwrap().is_none());
}

#[test]
fn refuses_unknown_members_shadowing_and_dynamic_access() {
    for source in [
        SIMPLE.replace("I.BaseInfo", "I.NotSupported"),
        SIMPLE.replace("I.BaseInfo", "I['BaseInfo']"),
        SIMPLE.replace("I.BaseInfo;", "const I = {};"),
        SIMPLE.replace("I.BaseInfo;", "I.BaseInfo; eval('1');"),
        SIMPLE.replace("I.BaseInfo;", "I.BaseInfo; globalThis[x];"),
        SIMPLE.replace("U.ObjectUtil.merge", "U.FetchUtil.get"),
        SIMPLE
            .replace("U.ObjectUtil.merge({}, {})", "U.Unknown")
            .replace("LightChaserUtil", "LightChaserUI"),
        SIMPLE.replace("globalThis.__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__ = C;", "") + "globalThis.__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__ = X;",
    ] {
        assert!(convert_entry(&source).is_err());
    }
}

#[test]
fn preserves_a_nested_function_parameter_with_the_same_name() {
    let source = SIMPLE.replace("I.BaseInfo;", "I.BaseInfo; function nested(I) { return I.Other; }");
    let converted = convert_entry(&source).unwrap().unwrap();
    assert!(converted.contains("function nested(I) { return I.Other; }"));
    let source = SIMPLE.replace("I.BaseInfo;", "I.BaseInfo; function nested() { var I = {}; return I.Other; }");
    let converted = convert_entry(&source).unwrap().unwrap();
    assert!(converted.contains("var I = {}; return I.Other;"));
}

#[test]
fn handles_base_map_and_rejects_unsupported_global_access() {
    let source = format!("const map = globalThis.__LCP_REMOTE_COMPONENT_BASE_URL_MAP__ || {{}};\n{SIMPLE}");
    let result = convert_entry(&source).unwrap().unwrap();
    assert!(result.contains("globalThis.__DATAVISOOL_REMOTE_COMPONENT_BASE_URL_MAP__"));
    assert!(convert_entry("window.LightChaserInterface.BaseInfo();").is_err());
}

#[test]
fn rejects_non_global_registration_and_module_only_entries() {
    let entry = "var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(I){return I.BaseInfo;}(LightChaserInterface);";
    assert!(convert_entry(&entry.replace("var ", "let ")).is_err());
    assert!(convert_entry(&format!("function enclosing(){{{entry}}}")).is_err());
    assert!(convert_entry(&format!("export default 1; {entry}")).is_err());
    let read = format!("{entry} console.log(__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__);");
    let result = convert_entry(&read).unwrap().unwrap();
    assert!(!result.contains("__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__"));
}

#[test]
fn accepts_static_window_global_arguments_but_rejects_dynamic_ones() {
    for arg in ["window.LightChaserUI", "globalThis['LightChaserUI']"] {
        let source = format!("var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){{return U.LCGUI;}}({arg});");
        assert!(convert_entry(&source).unwrap().unwrap().contains("DataVisoolUI"));
    }
    let dynamic = "var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){return U.LCGUI;}(window[which]);";
    assert!(convert_entry(dynamic).is_err());
}

#[test]
fn maps_reviewed_ui_sdk_types_and_preserves_web_merge_semantics() {
    let source = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(I,U,V,S,T){
            return [I.AbstractDesignerDefinition, U.ObjectUtil.merge({}, {x:{}}), V.LCGUI, V.Button, S.IdGenerate, T.DesignerMode];
        }(LightChaserInterface,LightChaserUtil,LCPUI,LCPSDK,LCPTypes);"#;
    let output = convert_entry(source).unwrap().unwrap();
    assert!(output.contains("DataVisoolSDK,DataVisoolSDK,DataVisoolUI,DataVisoolSDK,DataVisoolTypes"));
    assert!(output.contains("V.DatavisoolGUI"));
    assert!(output.contains("datavisoolLegacyMerge"));
    assert!(output.contains("__DATAVISOOL_REMOTE_COMPONENT_TEMP__"));
}

#[test]
fn applies_cli_rules_without_version_metadata_and_rejects_unknown_sdk_members() {
    let known = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(S,T){return [S.IdGenerate,T.DesignerMode];}(LCPSDK,LCPTypes);"#;
    let converted = convert_entry(known).unwrap().unwrap();
    assert!(converted.contains("DataVisoolSDK,DataVisoolTypes"));
    for source in [known.replace("S.IdGenerate", "S.UnknownMember"), known.replace("T.DesignerMode", "T.UnknownMember")] {
        let error = convert_entry(&source).unwrap_err();
        assert!(error.contains("UnknownMember"), "{error}");
    }
}

#[test]
fn converts_react_only_and_dependency_free_components() {
    for source in [
        "var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(R){return R.createElement;}(React);",
        "(function(){globalThis.__LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(){};})();",
    ] {
        let output = convert_entry(source).unwrap().unwrap();
        assert!(output.contains("__DATAVISOOL_REMOTE_COMPONENT_TEMP__"));
    }
}

#[test]
fn converts_legacy_cli_bootstrap_but_rejects_altered_prefixes() {
    for source in [
        include_str!("fixtures/legacy-cli-component.js"),
        include_str!("fixtures/legacy-cli-built-component.js"),
        include_str!("fixtures/legacy-cli-sdk-component.js"),
    ] {
        let output = convert_entry(source).unwrap().unwrap();
        assert!(output.contains("__DATAVISOOL_REMOTE_COMPONENT_TEMP__"));
        assert!(!output.contains("globalThis.__LCP_REMOTE_COMPONENT_BASE_URL_MAP__"));
        assert!(output.contains("data-datavisool-remote-style"));
        assert!(convert_entry(&source.replace("document.currentScript", "document.otherScript")).is_err());
    }
}

#[test]
fn converts_known_direct_class_names_and_rejects_ambiguous_references() {
    let source = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){return {className:'lcp-button custom'};}(LCPUI);"#;
    let output = convert_entry(source).unwrap().unwrap();
    assert!(output.contains("datavisool-button custom"));
    assert!(convert_entry(&source.replace("className:", "other:")).is_err());
    assert!(convert_entry(&source.replace("className:'lcp-button custom'", "className:`lcp-button`")).is_err());
}

#[test]
fn recognizes_escaped_legacy_identifiers_and_ignores_comments() {
    let source = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(I){return I.BaseInfo;}(\u004cightChaserInterface);"#;
    assert!(convert_entry(source).unwrap().unwrap().contains("DataVisoolSDK"));
    assert!(convert_entry("// LightChaserUI\nconst example = 1;").unwrap().is_none());
}

#[test]
fn converts_direct_old_style_literals_without_a_legacy_sdk_dependency() {
    let source = "const x = {className: 'lcp-button custom'};";
    assert!(convert_entry(source).unwrap().unwrap().contains("datavisool-button custom"));
    let inline_css = "const style=document.createElement('style');style.style.cssText='.lcp-button{color:red}';";
    assert!(convert_entry(inline_css)
        .unwrap()
        .unwrap()
        .contains(".datavisool-button{color:red}"));
    assert!(convert_entry("// LightChaserUI and lcp-button\nconst x = 1;")
        .unwrap()
        .is_none());
    assert!(convert_entry("const x = '/assets/lcp-button.svg';").unwrap().is_none());
    assert!(convert_entry("const x = getClass('lcp-button');").is_err());
}

#[test]
fn prefilters_escaped_css_without_parsing_unrelated_modern_scripts() {
    let escaped_css = r#"const style=document.createElement('style');style.style.cssText='.\\6c cp-button{}';"#;
    assert!(convert_entry(escaped_css).unwrap().unwrap().contains(".datavisool-button{}"));
    let escaped_parts = r#"const style=document.createElement('style');style.style.cssText='.lc\\70 -button{--lc\\70 -scrollbar-thumb:red}.lcp\\2d button{color:red}';"#;
    let converted = convert_entry(escaped_parts).unwrap().unwrap();
    assert!(converted.contains(".datavisool-button{--datavisool-scrollbar-thumb:red}"));
    assert!(converted.contains(".datavisool-button{color:red}"));
    let class_names = "const button={className:'lcp-button custom'};const editor={className:'cm-s-lcp-dark'};";
    let converted = convert_entry(class_names).unwrap().unwrap();
    assert!(converted.contains("datavisool-button custom"));
    assert!(converted.contains("cm-s-datavisool-dark"));

    let modern = r#"const matcher = /[a&&b]/v; const path = "C:\\tmp";"#;
    assert!(convert_entry(modern).unwrap().is_none());
    let large = r#"const path = "C:\\tmp";"#.repeat(400_000);
    assert!(large.len() > 4 * 1024 * 1024);
    assert!(convert_entry(&large).unwrap().is_none());
}

#[test]
fn converts_static_style_text_only_when_the_element_binding_is_proven() {
    let text_content = r#"const styleEl=document.createElement('style');styleEl.textContent='.\\6c cp-button{}';"#;
    let parsed = biome_js_parser::parse_script(text_content, biome_js_parser::JsParserOptions::default());
    assert!(super::proven_style_element_binding(&parsed.syntax(), "styleEl"));
    assert!(convert_entry(text_content).unwrap().unwrap().contains(".datavisool-button{}"));
    let inner_html = r#"const styleEl=document.createElement("style");styleEl.innerHTML='[class~=lcp-button]{}';"#;
    assert!(convert_entry(inner_html)
        .unwrap()
        .unwrap()
        .contains("[class~=datavisool-button]{}"));
    let inner_text = r#"const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{color:rgb(1,2,3)}';document.head.appendChild(styleEl);document.querySelector('body');document.getElementById('target');"#;
    assert!(convert_entry(inner_text)
        .unwrap()
        .unwrap()
        .contains("innerText=\".datavisool-button{color:rgb(1,2,3)}\""));

    let two_styles = r#"const first=document.createElement('style');first.innerText='.lcp-button{color:red}';document.head.appendChild(first);const second=document.createElement('style');second.innerText='.lcp-checkbox{color:blue}';document.head.appendChild(second);"#;
    let converted = convert_entry(two_styles).unwrap().unwrap();
    assert!(converted.contains("first.innerText=\".datavisool-button{color:red}\""));
    assert!(converted.contains("second.innerText=\".datavisool-checkbox{color:blue}\""));

    let local_window_shadow = r#"function unrelated(window){return window.location;}const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.append(styleEl);"#;
    assert!(convert_entry(local_window_shadow)
        .unwrap()
        .unwrap()
        .contains(".datavisool-button{}"));

    for instanceof_checks in [
        r#"const isElement=node instanceof window.Element;const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.append(styleEl);"#,
        r#"const isDocument=node instanceof Document;const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.append(styleEl);"#,
        r#"const hasOwn=Object.prototype.hasOwnProperty.call({x:1},'x');const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.append(styleEl);"#,
    ] {
        assert!(
            convert_entry(instanceof_checks).is_ok(),
            "rejected safe type check or Object.prototype use: {instanceof_checks}"
        );
    }

    let chained_instanceof = r#"const isElement=node instanceof window.dom.Element;const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.append(styleEl);"#;
    assert!(convert_entry(chained_instanceof).is_err(), "accepted a chained, unverified instanceof constructor");

    let multi_append = r#"const first=document.createElement('style');first.innerText='.lcp-button{color:red}';const second=document.createElement('style');second.innerText='.lcp-checkbox{color:blue}';document.head.append(first,second);"#;
    let converted = convert_entry(multi_append).unwrap().unwrap();
    assert!(converted.contains("document.head.append(first,second)"));
    assert!(converted.contains("first.innerText=\".datavisool-button{color:red}\""));
    assert!(converted.contains("second.innerText=\".datavisool-checkbox{color:blue}\""));

    let mut many_styles = String::new();
    for index in 0..70 {
        many_styles.push_str(&format!("const styleEl{index}=document.createElement('style');styleEl{index}.innerText='.lcp-button{{color:red}}';document.head.appendChild(styleEl{index});"));
    }
    let converted = convert_entry(&many_styles).unwrap().unwrap();
    assert!(converted.contains("styleEl69.innerText=\".datavisool-button{color:red}\""));

    for source in [
        "let styleEl=document.createElement('style');styleEl.textContent='.lcp-button{}';",
        "const styleEl=document.createElement('div');styleEl.textContent='.lcp-button{}';",
        "const styleEl=document.createElement('style');styleEl.textContent='.lcp-button{}';styleEl=document.createElement('style');",
        "const styleEl=document.createElement('style');function set(styleEl){styleEl.textContent='.lcp-button{}';}",
        "function f(document){const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.appendChild(styleEl);}",
        "document.createElement=function(){return {};};const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.appendChild=function(){};document.head.appendChild(styleEl);",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.querySelector('body').ownerDocument.createElement=function(){};document.head.appendChild(styleEl);",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';globalThis.document.createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';window['document'].createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';const w=window;w.document.createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';parent.document.createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';self.document.createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';top.document.createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';this.document.createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';window[key].createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';Node.prototype.appendChild=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';Element.prototype.append=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';Document.prototype.createElement=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';const proto=Element.prototype;proto.appendChild=function(){};",
        "const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';const E=window.Element;E.prototype.append=function(){};",
        "const proto=Object.getPrototypeOf(Object.getPrototypeOf(document.createElement('div')));proto.innerText=function(){};const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.append(styleEl);",
        "const proto=document.createElement('div').__proto__.__proto__;proto.innerText=function(){};const styleEl=document.createElement('style');styleEl.innerText='.lcp-button{}';document.head.append(styleEl);",
    ] {
        assert!(convert_entry(source).is_err(), "accepted ambiguous style assignment: {source}");
    }
}

#[test]
fn converts_only_static_css_strings_in_recognized_script_contexts() {
    let source = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(R,U){
            const style=document.createElement('style');
            style.style.cssText='.lcp-button{--lcp-scrollbar-thumb:var(--lcp-scrollbar-thumb)}';
            const note='The .custom-button token in this business string is not CSS';
            return [R.createElement('style',null,`.cm-s-lcp-dark{color:red}`),note];
        }(React,LightChaserUI);"#;
    let output = convert_entry(source).unwrap().unwrap();
    assert!(output.contains(".datavisool-button{--datavisool-scrollbar-thumb:var(--datavisool-scrollbar-thumb)}"));
    assert!(output.contains(".cm-s-datavisool-dark{color:red}"));
    assert!(output.contains("The .custom-button token in this business string is not CSS"));
    let arrow_alias = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const style=(R=>R.createElement('style',null,'.lcp-button{}'))(React);return [style,U.LCGUI];}(LightChaserUI);"#;
    assert!(convert_entry(arrow_alias).unwrap().unwrap().contains(".datavisool-button{}"));

    let dynamic = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const color='red';const style=document.createElement('style');style.style.cssText=`.lcp-button{color:${color}}`;return U.LCGUI;}(LightChaserUI);"#;
    let dynamic_error = convert_entry(dynamic).unwrap_err();
    assert!(dynamic_error.contains("动态脚本样式模板"), "{dynamic_error}");
    let escaped_dynamic = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const color='red';const style=document.createElement('style');style.style.cssText=`.\\6c cp-button{color:${color}}`;return U.LCGUI;}(LightChaserUI);"#;
    assert!(convert_entry(escaped_dynamic).unwrap_err().contains("动态脚本样式模板"));

    let unrelated_factory =
        r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){return U.createElement('style',null,'.lcp-button{}');}(LightChaserUI);"#;
    assert!(convert_entry(unrelated_factory).is_err());
}

#[test]
fn protects_utf16_surrogate_pairs_from_parser_panic_without_changing_output_text() {
    let source = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){
            const value='\uD83D\uDE00';
            const raw=String.raw`\uD83D\uDE00`;
            const regex=/[a-z]+/;
            // comment with ordinary text
            const escaped='\\uD83D\\uDE00';
            return [value,raw,regex,escaped,U.LCGUI];
        }(LightChaserUI);"#;
    let shadow = super::source_safety::parser_source(source).unwrap();
    assert_eq!(shadow.len(), source.len());
    assert!(shadow.contains("\\u{01f600}\\\n"));
    assert!(shadow.contains("const regex=/[a-z]+/;"));
    assert!(shadow.contains("// comment with ordinary text"));
    assert!(shadow.contains("const escaped='\\\\uD83D\\\\uDE00';"));

    let converted = convert_entry(source).unwrap().unwrap();
    for unchanged in [
        "const value='\\uD83D\\uDE00';",
        "String.raw`\\uD83D\\uDE00`",
        "const regex=/[a-z]+/;",
        "// comment with ordinary text",
        "const escaped='\\\\uD83D\\\\uDE00';",
    ] {
        assert!(converted.contains(unchanged), "missing original text: {unchanged}");
    }
    assert!(converted.contains("DataVisoolUI"));

    let css = r#"const style=document.createElement('style');style.style.cssText='.lcp-button{content:"\uD83D\uDE00"}';"#;
    let converted_css = convert_entry(css).unwrap().unwrap();
    assert!(converted_css.contains(".datavisool-button{content:\\\"😀\\\"}"), "{converted_css}");

    let unpaired = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const value='\uD83D';return U.LCGUI;}(LightChaserUI);"#;
    assert!(convert_entry(unpaired).unwrap_err().contains("未配对的 UTF-16 代理项"));
    let identifier = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const \uD801\uDC00=1;return U.LCGUI;}(LightChaserUI);"#;
    assert!(convert_entry(identifier).unwrap_err().contains("标识符中的 UTF-16 代理项"));
    let regex =
        r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const value=/[\uD800-\uDBFF][\uDC00-\uDFFF]/g;return U.LCGUI;}(LightChaserUI);"#;
    let regex_shadow = super::source_safety::parser_source(regex).unwrap();
    assert_eq!(regex_shadow.len(), regex.len());
    assert!(regex_shadow.contains(r#"/[\uE000-\uE000][\uE000-\uE000]/g"#));
    assert!(!regex_shadow.contains(r#"\uD800"#));
    let converted_regex = convert_entry(regex).unwrap().unwrap();
    assert!(converted_regex.contains(r#"/[\uD800-\uDBFF][\uDC00-\uDFFF]/g"#));

    // The real legacy antd-slider bundle uses this regular expression to
    // recognize UTF-16 surrogate pairs; the importer must not reject it.
    let antd_regex =
        r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){var l=/[\uD800-\uDBFF][\uDC00-\uDFFF]/g;return U.LCGUI;}(LightChaserUI);"#;
    let converted_antd_regex = convert_entry(antd_regex).unwrap().unwrap();
    assert!(converted_antd_regex.contains(r#"/[\uD800-\uDBFF][\uDC00-\uDFFF]/g"#));
    assert!(converted_antd_regex.contains("DataVisoolUI"), "{converted_antd_regex}");
    let class_name = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){return {className:'lcp-\u0062utton'};}(LightChaserUI);"#;
    assert!(convert_entry(class_name).unwrap().unwrap().contains("datavisool-button"));
}

#[test]
fn parser_surrogate_inputs_are_isolated_from_non_unwinding_panics() {
    const CHILD: &str = "DATAVISOOL_LEGACY_SURROGATE_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let valid_pair =
            r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const value='\uD83D\uDE00';return [value,U.LCGUI];}(LightChaserUI);"#;
        assert!(convert_entry(valid_pair).unwrap().unwrap().contains("DataVisoolUI"));
        let valid_template =
            r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const value=`\uD83D\uDE00`;return [value,U.LCGUI];}(LightChaserUI);"#;
        assert!(convert_entry(valid_template).unwrap().unwrap().contains("DataVisoolUI"));
        let valid_regex = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const regex=/[\uD800-\uDBFF][\uDC00-\uDFFF]/g;return [regex,U.LCGUI];}(LightChaserUI);"#;
        assert!(convert_entry(valid_regex)
            .unwrap()
            .unwrap()
            .contains(r#"/[\uD800-\uDBFF][\uDC00-\uDFFF]/g"#));
        for value in [r#"'\u{D800}'"#, r#"'\u{DFFF}'"#, r#"`\u{D800}`"#, r#"`\u{DFFF}`"#, r#"\u{D800}"#, r#"\u{DFFF}"#] {
            let source = format!("var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){{const value={value};return U.LCGUI;}}(LightChaserUI);");
            assert!(convert_entry(&source).is_err());
        }
        for identifier in [r#"\u{D800}"#, r#"\u{DFFF}"#] {
            let source = format!("var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){{const {identifier}=1;return U.LCGUI;}}(LightChaserUI);");
            assert!(convert_entry(&source).is_err());
        }
        let update_division = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const x=n++ / "\uD800" / 2;return U.LCGUI;}(LightChaserUI);"#;
        assert!(convert_entry(update_division).is_err());
        let division_with_comment_marker =
            r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){const x=n++ / "http://\uD800" / 2;return U.LCGUI;}(LightChaserUI);"#;
        assert!(convert_entry(division_with_comment_marker).is_err());
        let commented_escape = r#"var __LIGHT_CHASER_REMOTE_COMPONENT_TEMP__=function(U){
                // \uD800
                return U.LCGUI;
            }(LightChaserUI);"#;
        assert!(convert_entry(commented_escape).is_err());
        return;
    }

    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("backend::routes::import::legacy_component::tests::parser_surrogate_inputs_are_isolated_from_non_unwinding_panics")
        .env(CHILD, "1")
        .status()
        .unwrap();
    assert!(status.success(), "parser child exited with {status}");
}

#[test]
fn converts_packaged_styles_and_keeps_the_result_idempotent() {
    let name = format!(
        "datavisool-legacy-package-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(name);
    std::fs::create_dir_all(root.join("assets")).unwrap();
    let js = include_str!("fixtures/legacy-cli-component.js");
    std::fs::write(root.join("chart.js"), js).unwrap();
    std::fs::write(root.join("assets/chart.css"), ".lcp-button{color:var(--lcp-grid-columns)}").unwrap();
    assert!(super::convert_package(&root, "chart.js").unwrap());
    assert!(std::fs::read_to_string(root.join("assets/chart.css"))
        .unwrap()
        .contains(".datavisool-button"));
    assert!(std::fs::read_to_string(root.join("chart.js")).unwrap().contains("DataVisoolUI"));
    assert!(!super::convert_package(&root, "chart.js").unwrap());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn package_scan_allows_large_current_assets_and_rejects_large_legacy_candidates() {
    use super::package_scan::{scan_large_source, ScanOutcome, SourceKind};
    let name = format!(
        "datavisool-large-package-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(name);
    std::fs::create_dir_all(&root).unwrap();

    let normal_js = r#"{ const path = "C:\\tmp"; }
"#
    .repeat(190_000)
        + "// LightChaserUI appears only in this comment\n";
    let normal_css = r#".custom{content:"C:\\tmp";background:url(http://a)}
"#
    .repeat(120_000);
    assert!(normal_js.len() > super::MAX_PACKAGE_CONVERT_BYTES as usize);
    assert!(normal_css.len() > super::MAX_PACKAGE_CONVERT_BYTES as usize);
    std::fs::write(root.join("current.js"), &normal_js).unwrap();
    std::fs::write(root.join("current.css"), &normal_css).unwrap();
    assert!(!super::convert_package(&root, "current.js").unwrap());
    assert_eq!(std::fs::metadata(root.join("current.js")).unwrap().len(), normal_js.len() as u64);
    assert_eq!(std::fs::metadata(root.join("current.css")).unwrap().len(), normal_css.len() as u64);

    let mut legacy_boundary = vec![b' '; 4 * 1024 * 1024 + 64];
    legacy_boundary[8190..8196].copy_from_slice(b"LCPSDK");
    let js_path = root.join("legacy-boundary.js");
    std::fs::write(&js_path, legacy_boundary).unwrap();
    assert_eq!(scan_large_source(&js_path, SourceKind::JavaScript).unwrap(), ScanOutcome::Candidate);

    let escaped_class = format!("{}const view={{className:'\\\\6c cp-button'}};", " ".repeat(4 * 1024 * 1024 + 8));
    let class_path = root.join("escaped-class.js");
    std::fs::write(&class_path, escaped_class).unwrap();
    assert_ne!(scan_large_source(&class_path, SourceKind::JavaScript).unwrap(), ScanOutcome::Clean);

    let escaped_attribute = format!("{}style.textContent='[class=\\\\6c cp-button]{{}}';", " ".repeat(4 * 1024 * 1024 + 8));
    let attribute_path = root.join("escaped-attribute.js");
    std::fs::write(&attribute_path, escaped_attribute).unwrap();
    assert_eq!(scan_large_source(&attribute_path, SourceKind::JavaScript).unwrap(), ScanOutcome::Candidate);

    let regex_old_api = format!("{}const x=/[//]/;LCPSDK.BaseInfo;", " ".repeat(4 * 1024 * 1024 + 8));
    let regex_path = root.join("regex-old-api.js");
    std::fs::write(&regex_path, regex_old_api).unwrap();
    assert_eq!(scan_large_source(&regex_path, SourceKind::JavaScript).unwrap(), ScanOutcome::Candidate);

    let css_escaped = format!("{} .\\\\6c cp-button{{--lc\\\\70 -scrollbar-thumb:red}}", " ".repeat(4 * 1024 * 1024 + 8));
    let css_path = root.join("escaped-style.css");
    std::fs::write(&css_path, css_escaped).unwrap();
    assert_eq!(scan_large_source(&css_path, SourceKind::Css).unwrap(), ScanOutcome::Candidate);

    let css_escape_boundary = format!(
        "{} .\\\\6c cp-button{{--lc\\\\70 -scrollbar-thumb:red}}{}",
        "x".repeat(8190),
        "x".repeat(4 * 1024 * 1024 + 64)
    );
    assert!(css_escape_boundary.len() > super::MAX_PACKAGE_CONVERT_BYTES as usize);
    std::fs::write(&css_path, css_escape_boundary).unwrap();
    assert_eq!(scan_large_source(&css_path, SourceKind::Css).unwrap(), ScanOutcome::Candidate);

    let css_attribute_gap = format!("{}[class{}=\\\\6c cp-button]{{}}", " ".repeat(4 * 1024 * 1024 + 8), " ".repeat(1024));
    std::fs::write(&css_path, css_attribute_gap).unwrap();
    assert_eq!(scan_large_source(&css_path, SourceKind::Css).unwrap(), ScanOutcome::Candidate);

    let css_url_then_old_selector = format!("{}x{{background:url(http://a)}}.lcp-button{{color:red}}", " ".repeat(4 * 1024 * 1024 + 8));
    std::fs::write(root.join("url-old-style.css"), css_url_then_old_selector).unwrap();
    assert!(super::convert_package(&root, "current.js").is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn package_scan_rejects_uncertain_unicode_escapes_and_respects_the_size_boundary() {
    use super::package_scan::{scan_large_source, ScanOutcome, SourceKind};
    let name = format!(
        "datavisool-large-boundary-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    let root = std::env::temp_dir().join(name);
    std::fs::create_dir_all(&root).unwrap();
    let exact = vec![b' '; super::MAX_PACKAGE_CONVERT_BYTES as usize];
    std::fs::write(root.join("exact.js"), &exact).unwrap();
    let above = vec![b' '; exact.len() + 1];
    std::fs::write(root.join("above.js"), &above).unwrap();
    assert!(!super::convert_package(&root, "exact.js").unwrap());
    assert_eq!(scan_large_source(&root.join("above.js"), SourceKind::JavaScript).unwrap(), ScanOutcome::Clean);

    let unicode = format!("{}{}", " ".repeat(exact.len() + 8), r"const value='\u004cightChaserUI';");
    let unicode_path = root.join("uncertain-escape.js");
    std::fs::write(&unicode_path, unicode).unwrap();
    assert_eq!(scan_large_source(&unicode_path, SourceKind::JavaScript).unwrap(), ScanOutcome::Uncertain);
    assert!(super::convert_package(&root, "exact.js").is_err());
    std::fs::remove_dir_all(root).unwrap();
}

#[test]
fn leaves_current_component_untouched() {
    assert!(convert_entry("globalThis.__DATAVISOOL_REMOTE_COMPONENT_TEMP__ = class {}; ")
        .unwrap()
        .is_none());
}
