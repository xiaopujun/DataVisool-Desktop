/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread 领域仓储的稳定错误边界。

use std::error::Error;
use std::fmt::{Display, Formatter};

#[derive(Debug)]
pub(crate) enum ThreadRepositoryError {
    Sql(sqlx::Error),
    Json(serde_json::Error),
    NotFound { entity: &'static str, key: String },
    ScopeMismatch { entity: &'static str, key: String },
    Conflict { code: &'static str, message: String },
    InvalidTransition(String),
    InvalidRecord(String),
}

impl ThreadRepositoryError {
    pub(crate) fn conflict(code: &'static str, message: impl Into<String>) -> Self {
        Self::Conflict { code, message: message.into() }
    }

    /// Returns whether this error is a transient SQLite write lock conflict.
    ///
    /// SQLite exposes both the primary result codes (`SQLITE_BUSY` = 5 and
    /// `SQLITE_LOCKED` = 6) and extended result codes whose low byte contains
    /// the corresponding primary code.  SQLx's SQLite driver returns the
    /// extended numeric code through `DatabaseError::code`, so checking the
    /// low byte covers both forms without retrying unrelated database errors.
    pub(crate) fn is_transient_write_conflict(&self) -> bool {
        match self {
            Self::Sql(error) => is_transient_sqlite_error(error),
            _ => false,
        }
    }

    pub(crate) fn stable_code(&self) -> &'static str {
        match self {
            Self::Sql(_) | Self::Json(_) => "agent.persistence_failed",
            Self::NotFound { .. } => "agent.not_found",
            Self::ScopeMismatch { .. } => "agent.scope_mismatch",
            Self::Conflict { code, .. } => code,
            Self::InvalidTransition(_) => "agent.invalid_transition",
            Self::InvalidRecord(_) => "agent.invalid_record",
        }
    }
}

fn is_transient_sqlite_error(error: &sqlx::Error) -> bool {
    let Some(code) = error.as_database_error().and_then(|database_error| database_error.code()) else {
        return false;
    };
    let Ok(code) = code.parse::<i64>() else {
        return false;
    };

    matches!(code & 0xff, 5 | 6)
}

impl Display for ThreadRepositoryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(error) => write!(formatter, "SQLite thread repository failed: {error}"),
            Self::Json(error) => write!(formatter, "thread JSON failed: {error}"),
            Self::NotFound { entity, key } => write!(formatter, "{entity} not found: {key}"),
            Self::ScopeMismatch { entity, key } => write!(formatter, "{entity} scope mismatch: {key}"),
            Self::Conflict { message, .. } | Self::InvalidTransition(message) | Self::InvalidRecord(message) => formatter.write_str(message),
        }
    }
}

impl Error for ThreadRepositoryError {}

impl From<sqlx::Error> for ThreadRepositoryError {
    fn from(error: sqlx::Error) -> Self {
        Self::Sql(error)
    }
}

impl From<serde_json::Error> for ThreadRepositoryError {
    fn from(error: serde_json::Error) -> Self {
        Self::Json(error)
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;
    use std::error::Error as StdError;
    use std::fmt::{Display, Formatter};

    use super::*;

    #[derive(Debug)]
    struct FakeDatabaseError {
        code: String,
    }

    impl Display for FakeDatabaseError {
        fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
            write!(formatter, "fake SQLite error ({})", self.code)
        }
    }

    impl StdError for FakeDatabaseError {}

    impl sqlx::error::DatabaseError for FakeDatabaseError {
        fn message(&self) -> &str {
            "fake SQLite error"
        }

        fn code(&self) -> Option<Cow<'_, str>> {
            Some(self.code.as_str().into())
        }

        fn as_error(&self) -> &(dyn StdError + Send + Sync + 'static) {
            self
        }

        fn as_error_mut(&mut self) -> &mut (dyn StdError + Send + Sync + 'static) {
            self
        }

        fn into_error(self: Box<Self>) -> Box<dyn StdError + Send + Sync + 'static> {
            self
        }

        fn kind(&self) -> sqlx::error::ErrorKind {
            sqlx::error::ErrorKind::Other
        }
    }

    fn sqlite_error(code: i64) -> ThreadRepositoryError {
        ThreadRepositoryError::Sql(sqlx::Error::database(FakeDatabaseError { code: code.to_string() }))
    }

    #[test]
    fn recognizes_primary_and_extended_sqlite_lock_codes() {
        for code in [5, 6, 261, 262, 517, 518, 773] {
            assert!(sqlite_error(code).is_transient_write_conflict(), "code {code} should be retryable");
        }
        for code in [0, 1, 4, 7, 19, 2067] {
            assert!(!sqlite_error(code).is_transient_write_conflict(), "code {code} should not be retryable");
        }
    }

    #[test]
    fn does_not_retry_non_sqlite_repository_errors() {
        assert!(!ThreadRepositoryError::Json(serde_json::from_str::<String>("{").unwrap_err()).is_transient_write_conflict());
        assert!(!ThreadRepositoryError::InvalidRecord("bad state".to_string()).is_transient_write_conflict());
    }
}
