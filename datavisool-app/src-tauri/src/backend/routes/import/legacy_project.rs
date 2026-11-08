/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
// LCP 项目数据只在导入边界转换为当前格式。
// TODO(legacy-compat): 停止支持 LCP 项目包后，移除旧压缩格式和旧组件配置转换；保留当前格式解析。

fn normalize_imported_project_json(input: &str) -> Result<String, String> {
    let decoded = if let Some(payload) = datavisool_compat::legacy_compressed_gzip_payload(input.trim()) {
        datavisool_codec::decompress_project_gzip(&format!("DVZ1:gzip:{payload}"))
            .map_err(|error| format!("导入失败：旧项目数据解压失败（{error}）"))?
    } else {
        input.to_string()
    };
    let mut value: Value = serde_json::from_str(&decoded)
        .map_err(|error| format!("导入失败：项目 JSON 解析失败（{error}）"))?;
    let original = value.clone();
    normalize_imported_component(&mut value);
    if value == original { return Ok(decoded); }
    serde_json::to_string(&value).map_err(|error| format!("导入失败：项目 JSON 序列化失败（{error}）"))
}

fn normalize_imported_component(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(normalize_imported_component),
        Value::Object(map) => {
            if map.get("sourceType").and_then(Value::as_str).is_some() {
                if let Some(data) = map.get_mut("staticData") {
                    if !data.is_null() && !data.as_object().is_some_and(|object| object.contains_key("value")) {
                        *data = serde_json::json!({"value": data.take()});
                    }
                }
            }
            let kind = map.get("base").and_then(|v| v.get("type")).and_then(Value::as_str).map(str::to_owned);
            if let Some(style) = map.get_mut("style").and_then(Value::as_object_mut) {
                match kind.as_deref() {
                    Some("LoopContainer") => {
                        if let Some(gap) = style.remove("gap") {
                            for name in ["horizontalGap", "verticalGap"] {
                                if style.get(name).map_or(true, Value::is_null) {
                                    style.insert(name.to_string(), gap.clone());
                                }
                            }
                        }
                    }
                    Some("LcBaseTable") => {
                        if let Some(body) = style.get_mut("body").and_then(Value::as_object_mut) {
                            for (current, old_names) in [
                                ("enableCarousel", &["autoScroll", "enableScroll", "enableInfiniteScroll", "infiniteScroll"][..]),
                                ("carouselSpeed", &["scrollSpeed", "speed"][..]),
                                ("carouselPauseSeconds", &["scrollPauseSeconds", "pauseSeconds"][..]),
                            ] {
                                if body.get(current).map_or(true, Value::is_null) {
                                    if let Some(old) = old_names.iter().find_map(|name| body.get(*name).filter(|v| !v.is_null()).cloned()) {
                                        let normalized = if current == "enableCarousel" {
                                            Value::Bool(matches!(old, Value::Bool(true)) || old == "true" || old == 1 || old == "1")
                                        } else if let Some(number) = old.as_f64().or_else(|| old.as_str().and_then(|s| s.parse::<f64>().ok())) {
                                            serde_json::Number::from_f64(number).map(Value::Number).unwrap_or(old)
                                        } else { old };
                                        body.insert(current.to_string(), normalized);
                                    }
                                }
                                for name in old_names { body.remove(*name); }
                            }
                        }
                    }
                    _ => {}
                }
            }
            for (key, child) in map {
                // 数据值是用户内容，不是项目配置；其中可能出现同名字段。
                if key != "value" && key != "staticData" {
                    normalize_imported_component(child);
                }
            }
        }
        _ => {}
    }
}
