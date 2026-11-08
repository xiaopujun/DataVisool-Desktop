/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! 项目导出接口。
//!
//! 负责把项目、资源文件、公共 API、公共数据库和远程组件打包成可导入的 zip。
//! 支持普通明文包，也支持 passphrase 加密包和 v3 项目图导出。

// 分文件维护同一模块的内部实现，保持导入和导出函数的共享作用域。
include!("export/manifest_and_datasources.rs");
include!("export/assets.rs");

#[cfg(test)]
mod tests {
    include!("export/tests.rs");
}
