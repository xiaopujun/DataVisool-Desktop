/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 项目导入接口。
//!
//! 负责读取项目导出包，校验 manifest，导入项目树、公共 API、
//! 公共数据库和静态资源，并在失败时回滚数据库与文件暂存结果。
//! 支持明文包、包级加密和 manifest 字段级加密。

mod legacy_component;

// 分文件维护同一模块的内部实现，保持导入和导出函数的共享作用域。
include!("import/manifest.rs");
include!("import/datasources.rs");
include!("import/legacy_project.rs");
include!("import/rewrite_and_staging.rs");
include!("import/assets.rs");

#[cfg(test)]
mod tests {
    include!("import/tests_manifest.rs");
    include!("import/tests_import.rs");
    include!("import/tests_legacy_project.rs");
    include!("import/tests_missing_datasources.rs");
}
