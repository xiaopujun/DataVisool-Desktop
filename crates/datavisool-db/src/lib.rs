/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
use sqlx::migrate::{MigrateError, Migrator};
use sqlx::SqlitePool;

pub async fn run_base_migrations(pool: &SqlitePool) -> Result<(), MigrateError> {
    let mut migrator = sqlx::migrate!("./migrations");
    migrator.set_ignore_missing(true);
    migrator.run(pool).await
}

pub async fn run_custom_migrations(
    pool: &SqlitePool,
    mut migrator: Migrator,
) -> Result<(), MigrateError> {
    migrator.set_ignore_missing(true);
    migrator.run(pool).await
}
