/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use crate::{DriverError, ErrorKind};
use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, SecondsFormat};
use serde_json::{Map, Number, Value};
use std::str::FromStr;
use tiberius::{ColumnData, ColumnType, FromSql, Row};
use tokio::time::Instant;

const CONVERSION_CHUNK_BYTES: usize = 64 * 1024;
const BASE64_CHUNK_BYTES: usize = CONVERSION_CHUNK_BYTES - (CONVERSION_CHUNK_BYTES % 3);
const CELLS_PER_YIELD: usize = 64;

pub(super) async fn rows_to_json(
    rows: Vec<Row>,
    deadline: Option<Instant>,
) -> Result<Value, DriverError> {
    let mut progress = ConversionProgress::new(deadline);
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        progress.check()?;
        let mut record = Map::new();
        for (column, data) in row.cells() {
            let value = column_to_json(column.column_type(), data, &mut progress).await?;
            record.insert(column.name().to_string(), value);
            progress.cell_completed().await?;
        }
        result.push(Value::Object(record));
        progress.row_completed()?;
    }
    progress.check()?;
    Ok(Value::Array(result))
}

struct ConversionProgress {
    deadline: Option<Instant>,
    cells_since_yield: usize,
}

impl ConversionProgress {
    fn new(deadline: Option<Instant>) -> Self {
        Self {
            deadline,
            cells_since_yield: 0,
        }
    }

    fn check(&self) -> Result<(), DriverError> {
        if self
            .deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            return Err(query_timeout_error());
        }
        Ok(())
    }

    async fn cell_completed(&mut self) -> Result<(), DriverError> {
        self.cells_since_yield += 1;
        if self.cells_since_yield >= CELLS_PER_YIELD {
            self.yield_and_check().await
        } else {
            self.check()
        }
    }

    fn row_completed(&self) -> Result<(), DriverError> {
        self.check()
    }

    async fn yield_and_check(&mut self) -> Result<(), DriverError> {
        self.check()?;
        tokio::task::yield_now().await;
        self.cells_since_yield = 0;
        self.check()
    }
}

async fn column_to_json(
    column_type: ColumnType,
    data: &ColumnData<'static>,
    progress: &mut ConversionProgress,
) -> Result<Value, DriverError> {
    progress.check()?;
    if matches!(column_type, ColumnType::Money | ColumnType::Money4) {
        return match data {
            ColumnData::F64(Some(value)) => money_to_json(*value),
            ColumnData::F32(Some(value)) => money_to_json(f64::from(*value)),
            ColumnData::F64(None) | ColumnData::F32(None) => Ok(Value::Null),
            _ => Err(conversion_error(
                "SQL Server money 列返回了非预期类型，无法保证小数精度",
            )),
        };
    }

    Ok(match data {
        ColumnData::U8(value) => value.map_or(Value::Null, |value| Value::Number(value.into())),
        ColumnData::I16(value) => value.map_or(Value::Null, |value| Value::Number(value.into())),
        ColumnData::I32(value) => value.map_or(Value::Null, |value| Value::Number(value.into())),
        ColumnData::I64(value) => value.map_or(Value::Null, |value| Value::Number(value.into())),
        ColumnData::F32(value) => {
            value.map_or(Value::Null, |value| float_to_json(f64::from(value)))
        }
        ColumnData::F64(value) => value.map_or(Value::Null, float_to_json),
        ColumnData::Bit(value) => value.map_or(Value::Null, Value::Bool),
        ColumnData::String(value) => match value.as_deref() {
            Some(value) => Value::String(copy_text(value, progress).await?),
            None => Value::Null,
        },
        ColumnData::Guid(value) => {
            value.map_or(Value::Null, |value| Value::String(value.to_string()))
        }
        ColumnData::Binary(value) => match value.as_deref() {
            Some(value) => Value::String(encode_base64(value, progress).await?),
            None => Value::Null,
        },
        ColumnData::Numeric(value) => match value {
            Some(value) => decimal_to_json(&value.to_string())?,
            None => Value::Null,
        },
        ColumnData::Xml(value) => match value.as_deref() {
            Some(value) => Value::String(copy_text(value.as_ref(), progress).await?),
            None => Value::Null,
        },
        ColumnData::DateTime(value) => match value {
            Some(_) => {
                let value = <NaiveDateTime as FromSql>::from_sql(data)
                    .map_err(conversion_error)?
                    .ok_or_else(|| conversion_error("SQL Server 日期值为空"))?;
                Value::String(value.to_string())
            }
            None => Value::Null,
        },
        ColumnData::SmallDateTime(value) => match value {
            Some(_) => {
                let value = <NaiveDateTime as FromSql>::from_sql(data)
                    .map_err(conversion_error)?
                    .ok_or_else(|| conversion_error("SQL Server 日期值为空"))?;
                Value::String(value.to_string())
            }
            None => Value::Null,
        },
        ColumnData::Time(value) => match value {
            Some(_) => {
                let value = <NaiveTime as FromSql>::from_sql(data)
                    .map_err(conversion_error)?
                    .ok_or_else(|| conversion_error("SQL Server 时间值为空"))?;
                Value::String(value.to_string())
            }
            None => Value::Null,
        },
        ColumnData::Date(value) => match value {
            Some(_) => {
                let value = <NaiveDate as FromSql>::from_sql(data)
                    .map_err(conversion_error)?
                    .ok_or_else(|| conversion_error("SQL Server 日期值为空"))?;
                Value::String(value.to_string())
            }
            None => Value::Null,
        },
        ColumnData::DateTime2(value) => match value {
            Some(_) => {
                let value = <NaiveDateTime as FromSql>::from_sql(data)
                    .map_err(conversion_error)?
                    .ok_or_else(|| conversion_error("SQL Server 日期值为空"))?;
                Value::String(value.to_string())
            }
            None => Value::Null,
        },
        ColumnData::DateTimeOffset(value) => match value {
            Some(_) => {
                let value = <DateTime<FixedOffset> as FromSql>::from_sql(data)
                    .map_err(conversion_error)?
                    .ok_or_else(|| conversion_error("SQL Server 日期值为空"))?;
                Value::String(value.to_rfc3339_opts(SecondsFormat::AutoSi, true))
            }
            None => Value::Null,
        },
    })
}

async fn copy_text(value: &str, progress: &mut ConversionProgress) -> Result<String, DriverError> {
    progress.check()?;
    if value.len() <= CONVERSION_CHUNK_BYTES {
        return Ok(value.to_owned());
    }

    let mut copied = String::with_capacity(value.len());
    let mut start = 0;
    while start < value.len() {
        let mut end = (start + CONVERSION_CHUNK_BYTES).min(value.len());
        while !value.is_char_boundary(end) {
            end -= 1;
        }
        copied.push_str(&value[start..end]);
        start = end;
        progress.yield_and_check().await?;
    }
    Ok(copied)
}

async fn encode_base64(
    value: &[u8],
    progress: &mut ConversionProgress,
) -> Result<String, DriverError> {
    progress.check()?;
    if value.len() <= BASE64_CHUNK_BYTES {
        return Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            value,
        ));
    }

    let encoded_capacity = value
        .len()
        .checked_add(2)
        .and_then(|length| (length / 3).checked_mul(4))
        .ok_or_else(|| conversion_error("SQL Server binary 列过大，无法转换为 Base64"))?;
    let mut encoded = String::with_capacity(encoded_capacity);
    let mut start = 0;
    while start < value.len() {
        let remaining = value.len() - start;
        let chunk_length = remaining.min(BASE64_CHUNK_BYTES);
        let end = start + chunk_length;
        encoded.push_str(&base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &value[start..end],
        ));
        start = end;
        progress.yield_and_check().await?;
    }
    Ok(encoded)
}

fn money_to_json(value: f64) -> Result<Value, DriverError> {
    const MAX_EXACT_SCALED_INTEGER: f64 = (1u64 << 52) as f64;
    const MONEY_SCALE: f64 = 10_000.0;
    if !value.is_finite() || value.abs() >= MAX_EXACT_SCALED_INTEGER / MONEY_SCALE {
        return Err(conversion_error(
            "SQL Server money 数值超出可安全恢复范围；请在 SQL 中使用 CAST(列名 AS DECIMAL(19,4))",
        ));
    }
    decimal_to_json(&format!("{value:.4}"))
}

fn decimal_to_json(value: &str) -> Result<Value, DriverError> {
    Number::from_str(value)
        .map(Value::Number)
        .map_err(|_| conversion_error("SQL Server decimal/numeric 无法精确转换为 JSON 数字"))
}

fn float_to_json(value: f64) -> Value {
    Number::from_f64(value)
        .map(Value::Number)
        .unwrap_or_else(|| Value::String(value.to_string()))
}

fn conversion_error(message: impl std::fmt::Display) -> DriverError {
    DriverError::new(ErrorKind::Query, message.to_string())
}

fn query_timeout_error() -> DriverError {
    DriverError::new(ErrorKind::Query, "SQL Server 查询超时")
}

#[cfg(test)]
mod tests {
    use super::{column_to_json, decimal_to_json, rows_to_json, ConversionProgress};
    use serde_json::Value;
    use std::{borrow::Cow, time::Duration};
    use tiberius::{ColumnData, ColumnType, Row};
    use tokio::time::Instant;

    #[tokio::test]
    async fn decimal_remains_an_exact_json_number() {
        let value = decimal_to_json("12345678901234567890.1234567890").unwrap();
        assert!(matches!(value, Value::Number(_)));
        assert_eq!(value.to_string(), "12345678901234567890.1234567890");
    }

    #[tokio::test]
    async fn nullable_and_binary_columns_have_stable_json_shapes() {
        let mut progress = ConversionProgress::new(None);
        assert_eq!(
            column_to_json(ColumnType::Int8, &ColumnData::I64(None), &mut progress)
                .await
                .unwrap(),
            Value::Null
        );
        assert_eq!(
            column_to_json(
                ColumnType::BigVarBin,
                &ColumnData::Binary(Some(Cow::Owned(vec![0, 1, 255]))),
                &mut progress,
            )
            .await
            .unwrap(),
            Value::String("AAH/".into())
        );
    }

    #[tokio::test]
    async fn money_keeps_four_decimal_places_for_safe_positive_and_negative_values() {
        let mut progress = ConversionProgress::new(None);
        assert_eq!(
            column_to_json(
                ColumnType::Money,
                &ColumnData::F64(Some(123.4567)),
                &mut progress,
            )
            .await
            .unwrap()
            .to_string(),
            "123.4567"
        );
        assert_eq!(
            column_to_json(
                ColumnType::Money4,
                &ColumnData::F64(Some(-12.3401)),
                &mut progress,
            )
            .await
            .unwrap()
            .to_string(),
            "-12.3401"
        );
        assert_eq!(
            column_to_json(ColumnType::Money, &ColumnData::F64(None), &mut progress)
                .await
                .unwrap(),
            Value::Null
        );
        assert_eq!(
            column_to_json(ColumnType::Money4, &ColumnData::F32(None), &mut progress)
                .await
                .unwrap(),
            Value::Null
        );
    }

    #[tokio::test]
    async fn money_at_i64_scaled_boundary_requires_decimal_cast() {
        let mut progress = ConversionProgress::new(None);
        let error = column_to_json(
            ColumnType::Money,
            &ColumnData::F64(Some(922_337_203_685_477.5807)),
            &mut progress,
        )
        .await
        .unwrap_err();
        assert!(error.message.contains("CAST(列名 AS DECIMAL(19,4))"));
    }

    #[tokio::test]
    async fn expired_deadline_stops_before_converting_real_row() {
        let row = Row::builder()
            .column(
                "payload",
                ColumnType::BigVarBin,
                ColumnData::Binary(Some(Cow::Owned(vec![0; 1024]))),
            )
            .build();
        let result = rows_to_json(vec![row], Some(Instant::now() - Duration::from_millis(1))).await;
        let error = result.expect_err("expired query deadline must stop conversion");
        assert_eq!(error.kind, crate::ErrorKind::Query);
        assert!(error.message.contains("查询超时"));
    }
}
