/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Thread 领域的 SQLite Repository。

mod common;
mod inbox;
mod read;
mod recovery;
mod resume_attempt;
mod thread_write;
mod turn_create;
mod turn_write;

use sqlx::SqlitePool;

#[derive(Clone)]
pub(crate) struct SqliteThreadRepository {
    pub(super) pool: SqlitePool,
}

impl SqliteThreadRepository {
    pub(crate) fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

pub(super) use common::{
    allocate_event_sequences, ensure_scope, event_from_row, find_deduplicated_event, insert_event, item_from_row, run_from_row, thread_from_row,
    thread_summary_from_row, turn_from_row, turn_input_from_row,
};
