/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Task 执行身份的格式校验。真实 ID 只能由持久化层生成或读取。

use super::TaskRepositoryError;

pub(super) fn validate_persisted_identity(project_id: i64, thread_id: &str, turn_id: &str, run_id: &str) -> Result<(), TaskRepositoryError> {
    if project_id <= 0 {
        return Err(TaskRepositoryError::InvalidRecord("project_id must be positive".to_string()));
    }
    for (field, value) in [("thread_id", thread_id), ("turn_id", turn_id), ("run_id", run_id)] {
        if value.trim().is_empty() {
            return Err(TaskRepositoryError::InvalidRecord(format!("{field} must not be blank")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_requires_real_scope_values() {
        assert!(validate_persisted_identity(1, "thr_random", "turn_random", "run_random").is_ok());
        assert!(validate_persisted_identity(0, "thr_random", "turn_random", "run_random").is_err());
        assert!(validate_persisted_identity(1, "", "turn_random", "run_random").is_err());
    }
}
