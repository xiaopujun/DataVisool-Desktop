/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};

use oracledb::{JsonValue as OracleJsonValue, Lob, OracleNumber, OracleTimestamp, Row};
use serde_json::{Map, Number, Value};

use crate::{DriverError, ErrorKind};

pub(super) fn cursor_to_json(
    cursor: oracledb::Cursor,
    cancelled: &AtomicBool,
) -> Result<Value, DriverError> {
    let columns = cursor
        .columns()
        .iter()
        .map(|column| {
            (
                column.name().to_string(),
                column.db_type().name().to_string(),
            )
        })
        .collect::<Vec<_>>();
    let mut result = Vec::new();

    for row_result in cursor {
        if is_cancelled(cancelled) {
            return Err(cancelled_error());
        }
        let row = row_result.map_err(|error| {
            DriverError::new(ErrorKind::Query, format!("读取Oracle查询结果失败：{error}"))
        })?;
        result.push(row_to_json(row, &columns, cancelled)?);
    }

    Ok(Value::Array(result))
}

fn row_to_json(
    mut row: Row,
    columns: &[(String, String)],
    cancelled: &AtomicBool,
) -> Result<Value, DriverError> {
    let mut values = Map::new();
    for (index, (name, type_name)) in columns.iter().enumerate() {
        if is_cancelled(cancelled) {
            return Err(cancelled_error());
        }
        let value = value_for_column(&mut row, index, name, type_name, cancelled)?;
        values.insert(name.clone(), value);
    }
    Ok(Value::Object(values))
}

fn value_for_column(
    row: &mut Row,
    index: usize,
    column_name: &str,
    type_name: &str,
    cancelled: &AtomicBool,
) -> Result<Value, DriverError> {
    match type_name {
        "DB_TYPE_CHAR"
        | "DB_TYPE_NCHAR"
        | "DB_TYPE_VARCHAR"
        | "DB_TYPE_NVARCHAR"
        | "DB_TYPE_LONG"
        | "DB_TYPE_LONG_NVARCHAR"
        | "DB_TYPE_ROWID"
        | "DB_TYPE_UROWID" => take::<Option<String>>(row, index, column_name).map(optional_value),
        "DB_TYPE_NUMBER" => {
            take::<Option<OracleNumber>>(row, index, column_name).and_then(|value| match value {
                Some(value) => number_to_json(value),
                None => Ok(Value::Null),
            })
        }
        "DB_TYPE_BINARY_FLOAT" => take::<Option<f32>>(row, index, column_name).map(|value| {
            value
                .map(|value| float_to_json(value as f64))
                .unwrap_or(Value::Null)
        }),
        "DB_TYPE_BINARY_DOUBLE" => take::<Option<f64>>(row, index, column_name)
            .map(|value| value.map(float_to_json).unwrap_or(Value::Null)),
        "DB_TYPE_BOOLEAN" => take::<Option<bool>>(row, index, column_name)
            .map(|value| value.map(Value::Bool).unwrap_or(Value::Null)),
        "DB_TYPE_RAW" | "DB_TYPE_LONG_RAW" => {
            take::<Option<Vec<u8>>>(row, index, column_name).map(|value| {
                value
                    .map(|bytes| {
                        Value::String(base64::Engine::encode(
                            &base64::engine::general_purpose::STANDARD,
                            bytes,
                        ))
                    })
                    .unwrap_or(Value::Null)
            })
        }
        "DB_TYPE_CLOB" | "DB_TYPE_NCLOB" => {
            let lob = take::<Option<Lob>>(row, index, column_name)?;
            match lob {
                Some(mut lob) => {
                    let bytes = read_lob(&mut lob, cancelled, column_name)?;
                    let text = String::from_utf8(bytes).map_err(|error| {
                        DriverError::new(
                            ErrorKind::Query,
                            format!("Oracle列{column_name}的CLOB不是有效UTF-8：{error}"),
                        )
                    })?;
                    Ok(Value::String(text))
                }
                None => Ok(Value::Null),
            }
        }
        "DB_TYPE_BLOB" => {
            let lob = take::<Option<Lob>>(row, index, column_name)?;
            match lob {
                Some(mut lob) => {
                    let bytes = read_lob(&mut lob, cancelled, column_name)?;
                    Ok(Value::String(base64::Engine::encode(
                        &base64::engine::general_purpose::STANDARD,
                        bytes,
                    )))
                }
                None => Ok(Value::Null),
            }
        }
        "DB_TYPE_DATE" | "DB_TYPE_TIMESTAMP" | "DB_TYPE_TIMESTAMP_LTZ" | "DB_TYPE_TIMESTAMP_TZ" => {
            take::<Option<OracleTimestamp>>(row, index, column_name).map(|value| {
                value
                    .map(|timestamp| timestamp_to_string(timestamp, type_name))
                    .map(Value::String)
                    .unwrap_or(Value::Null)
            })
        }
        "DB_TYPE_INTERVAL_DS" => {
            take::<Option<oracledb::OracleIntervalDS>>(row, index, column_name).map(|value| {
                value
                    .map(|interval| Value::String(interval.to_string()))
                    .unwrap_or(Value::Null)
            })
        }
        "DB_TYPE_INTERVAL_YM" => {
            take::<Option<oracledb::OracleIntervalYM>>(row, index, column_name).map(|value| {
                value
                    .map(|interval| Value::String(interval.to_string()))
                    .unwrap_or(Value::Null)
            })
        }
        "DB_TYPE_JSON" => match take::<Option<OracleJsonValue>>(row, index, column_name)? {
            Some(value) => json_value_to_value(value),
            None => Ok(Value::Null),
        },
        "DB_TYPE_BFILE" | "DB_TYPE_CURSOR" | "DB_TYPE_OBJECT" | "DB_TYPE_VECTOR"
        | "DB_TYPE_UNKNOWN" => Err(unsupported_type(column_name, type_name)),
        _ => Err(unsupported_type(column_name, type_name)),
    }
}

fn take<T>(row: &mut Row, index: usize, column_name: &str) -> Result<T, DriverError>
where
    for<'a> T: oracledb::FromDbValue<'a>,
{
    row.take(index).map_err(|error| {
        DriverError::new(
            ErrorKind::Query,
            format!("转换Oracle列{column_name}的值失败：{error}"),
        )
    })
}

fn optional_value(value: Option<String>) -> Value {
    value.map(Value::String).unwrap_or(Value::Null)
}

pub(super) fn number_to_json(value: OracleNumber) -> Result<Value, DriverError> {
    let text = value.to_string();
    text.parse::<Number>().map(Value::Number).map_err(|error| {
        DriverError::new(
            ErrorKind::Query,
            format!("Oracle NUMBER无法转换为JSON数字：{error}"),
        )
    })
}

fn float_to_json(value: f64) -> Value {
    Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(value.to_string()))
}

pub(super) fn timestamp_to_string(value: OracleTimestamp, type_name: &str) -> String {
    let date = format!(
        "{:04}-{:02}-{:02}",
        value.year(),
        value.month(),
        value.day()
    );
    if type_name == "DB_TYPE_DATE" {
        return format!(
            "{date}T{:02}:{:02}:{:02}",
            value.hour(),
            value.minute(),
            value.second()
        );
    }

    let mut timestamp = format!(
        "{date}T{:02}:{:02}:{:02}",
        value.hour(),
        value.minute(),
        value.second()
    );
    if value.nanoseconds() != 0 {
        timestamp.push_str(&format!(".{:09}", value.nanoseconds()));
    }
    if type_name == "DB_TYPE_TIMESTAMP_TZ" {
        let offset = i16::from(value.tz_hour_offset()) * 60 + i16::from(value.tz_minute_offset());
        let sign = if offset < 0 { '-' } else { '+' };
        let absolute_offset = offset.abs();
        timestamp.push_str(&format!(
            "{sign}{:02}:{:02}",
            absolute_offset / 60,
            absolute_offset % 60
        ));
    }
    timestamp
}

fn read_lob(
    lob: &mut Lob,
    cancelled: &AtomicBool,
    column_name: &str,
) -> Result<Vec<u8>, DriverError> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        if is_cancelled(cancelled) {
            return Err(cancelled_error());
        }
        let count = lob.read(&mut buffer).map_err(|error| {
            DriverError::new(
                ErrorKind::Query,
                format!("读取Oracle列{column_name}的LOB失败：{error}"),
            )
        })?;
        if count == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(bytes)
}

fn json_value_to_value(value: OracleJsonValue) -> Result<Value, DriverError> {
    Ok(match value {
        OracleJsonValue::BinaryDouble(value) => float_to_json(value),
        OracleJsonValue::BinaryFloat(value) => float_to_json(value as f64),
        OracleJsonValue::Boolean(value) => Value::Bool(value),
        OracleJsonValue::IntervalDS(value) => Value::String(value.to_string()),
        OracleJsonValue::IntervalYM(value) => Value::String(value.to_string()),
        OracleJsonValue::JsonArray(values) => Value::Array(
            values
                .into_iter()
                .map(json_value_to_value)
                .collect::<Result<Vec<_>, _>>()?,
        ),
        OracleJsonValue::JsonId(value) | OracleJsonValue::Raw(value) => Value::String(
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, value),
        ),
        OracleJsonValue::JsonObject(values) => {
            let mut object = Map::new();
            for (key, value) in values {
                object.insert(key, json_value_to_value(value)?);
            }
            Value::Object(object)
        }
        OracleJsonValue::Null => Value::Null,
        OracleJsonValue::Number(value) => number_to_json(value)?,
        OracleJsonValue::String(value) => Value::String(value),
        OracleJsonValue::Timestamp(value) => Value::String(value.to_string()),
        OracleJsonValue::Vector(_) => {
            return Err(unsupported_type("JSON值", "Oracle JSON VECTOR"));
        }
    })
}

fn unsupported_type(column_name: &str, type_name: &str) -> DriverError {
    DriverError::new(
        ErrorKind::Query,
        format!("Oracle列{column_name}的数据类型{type_name}暂不支持导出"),
    )
}

fn is_cancelled(cancelled: &AtomicBool) -> bool {
    cancelled.load(Ordering::Acquire)
}

fn cancelled_error() -> DriverError {
    DriverError::new(ErrorKind::Query, "Oracle操作已超时并取消后续结果读取")
}
