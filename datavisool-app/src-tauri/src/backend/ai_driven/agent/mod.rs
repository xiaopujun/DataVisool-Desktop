/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
//! Rig Agent 运行期的项目级约束。

mod argument_shape;
mod completion;
mod context_compaction;
mod context_history;
mod context_plan;
mod factory;
mod history;
mod hooks;
mod item_state;
mod limits;
mod output;
mod progress_guard;
mod runner;
mod steering;
mod tool_concurrency;
mod tool_failure_policy;
mod tool_route;
mod tool_timeout;

pub(crate) use context_plan::AgentContextPlan;
pub(crate) use factory::{AgentBuildRequest, RigAgentFactory};
pub(crate) use hooks::AgentRuntimeHook;
pub(crate) use tool_route::AgentToolRoute;

pub(crate) use completion::{AgentCompletionDecision, AgentCompletionGate, CompletionGateFuture};

pub(crate) use limits::{AgentContextLimits, AgentExecutionLimits, ExecutionBudget, ExecutionCancellationToken, ExecutionUsageSnapshot};
pub(crate) use output::{AgentRunFailure, AgentRunOutput};
pub(crate) use runner::run_built_agent;
pub(crate) use steering::AgentSteeringHook;
