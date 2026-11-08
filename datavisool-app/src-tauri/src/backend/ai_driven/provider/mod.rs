/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig 模型 Provider 适配层。
//!
//! 本模块只负责把项目运行配置转换为具体 Rig 模型，不处理会话、附件或 Agent 循环。

mod config;
mod factory;
mod observed_model;

#[cfg(test)]
mod compatibility_tests;

pub(crate) use factory::{BuiltRigModel, RigCompletionModel, RigModelFactory};
pub(crate) use observed_model::{ContextTraceIdentity, ObservedCompletionModel};
