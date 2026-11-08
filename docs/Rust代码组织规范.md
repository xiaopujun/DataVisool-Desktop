# Rust 代码组织规范

本文用于约束本仓库 Rust 代码的目录拆分、模块边界、命名、可见性、错误处理和测试组织。目标是让 `datavisool-app/src-tauri` 和 `crates/*` 在长期演进中保持一致的工程结构。

## 1. 总体原则

Rust 模块组织优先采用“按业务能力纵向拆分”的方式，而不是按技术层统一堆叠。一个业务能力目录内应包含该能力需要的类型、服务、仓储、路由、错误和测试，调用方通过该能力模块暴露的稳定接口使用它。

推荐方向：

```text
src/
├── auth/
│   ├── mod.rs
│   ├── types.rs
│   ├── service.rs
│   ├── repository.rs
│   └── errors.rs
├── project/
│   ├── mod.rs
│   ├── types.rs
│   ├── service.rs
│   ├── repository.rs
│   └── errors.rs
├── ai_agent/
│   ├── mod.rs
│   ├── types.rs
│   ├── planner.rs
│   ├── executor.rs
│   └── errors.rs
├── app.rs
├── config.rs
├── error.rs
├── state.rs
└── main.rs
```

核心原则：

- `main.rs` 只负责启动流程，避免承载业务逻辑。
- `mod.rs` 是模块门面，负责声明子模块和导出必要 API。
- 业务能力内部默认私有，只有真正给外部模块使用的类型和函数才 `pub`。
- 服务层负责编排业务用例，仓储层只负责数据读写，路由或 Tauri command 只做协议适配。
- 跨多个业务领域复用的能力才上升为 `src/` 下的公共模块或独立 crate。
- 避免长期维护巨大的 `types.rs`、`utils.rs`、`service.rs`、`routes.rs` 单文件。

## 2. Crate 与模块边界

### 2.1 什么时候新建 crate

只有满足以下条件之一时，才考虑在 Cargo workspace 中新建 crate：

- 能力需要被多个模块或程序共同复用。
- 能力有清晰稳定的输入输出边界，例如数据库访问、项目格式、归档、编码、资产处理。
- 能力可以独立测试、独立发布或独立进行性能优化。
- 能力不依赖上层应用状态、HTTP 框架、Tauri API 或具体 UI 语义。

本仓库中类似 `crates/datavisool-db`、`crates/datavisool-archive`、`crates/datavisool-project-format` 的基础库适合保留为 crate。

### 2.2 什么时候新建 feature module

满足以下条件时，应在当前 crate 的 `src/` 下新建业务模块目录：

- 它代表一个明确业务域，例如 `auth`、`project`、`license`、`import`、`ai_agent`。
- 它需要多个协作文件才能表达完整逻辑。
- 它有独立的数据类型、错误、服务方法或路由入口。
- 它未来可能增长，不适合继续放在单个平铺文件里。

### 2.3 什么时候只建单文件模块

以下情况可以使用 `foo.rs` 单文件模块：

- 代码量很小，预计不会快速扩张。
- 只提供一个清晰的基础能力，例如 `config.rs`、`state.rs`、`http.rs`。
- 文件内没有混合多个业务域。

当单文件模块出现多组类型、多组错误、多组外部入口或超过约 300 行时，应评估迁移到目录模块。

## 3. 推荐目录职责

业务模块目录推荐使用以下文件名。不是每个模块都必须拥有全部文件，应按实际复杂度裁剪。

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 声明子模块、导出模块对外 API、提供模块级初始化函数。 |
| `types.rs` | 放置该业务域的领域类型、DTO、请求响应结构和枚举。 |
| `service.rs` | 放置业务用例编排、校验、事务边界和跨仓储协调逻辑。 |
| `repository.rs` | 放置数据库、文件系统或外部持久化读写逻辑。 |
| `routes.rs` | Axum 等 HTTP 路由入口和 handler 适配层。 |
| `commands.rs` | Tauri command 入口和桌面端协议适配层。 |
| `errors.rs` | 该业务域错误枚举、Result 别名和错误转换。 |
| `client.rs` | 第三方 HTTP、SDK 或外部服务客户端封装。 |
| `mapper.rs` | 数据库行、外部响应、内部领域类型之间的转换。 |
| `validation.rs` | 复杂输入校验、规格约束和边界归一化。 |
| `tests.rs` | 模块级单元测试；集成测试更推荐放到 crate 的 `tests/` 目录。 |

## 4. `mod.rs` 写法

`mod.rs` 应作为模块门面，调用方不应该直接依赖模块内部的文件结构。

推荐写法：

```rust
mod errors;
mod repository;
mod service;
mod types;

pub use errors::{ProjectError, ProjectResult};
pub use service::ProjectService;
pub use types::{CreateProjectRequest, ProjectSummary};
```

如果某些子模块只供当前业务模块内部使用，保持 `mod xxx;` 即可，不要 `pub mod xxx;`。

不推荐写法：

```rust
pub mod errors;
pub mod repository;
pub mod service;
pub mod types;
```

这种写法会把内部结构暴露给外部调用方，后续重构成本高。

## 5. 文件分层规则

### 5.1 `types.rs`

`types.rs` 放置该业务模块自己的类型。类型应按用途分组：

- Request：外部输入，例如 HTTP body、query、Tauri command 参数。
- Response：外部输出，例如接口响应、前端消费 DTO。
- Domain：服务层内部使用的领域模型。
- Config：模块局部配置。
- Enum：业务状态、类型标识和受控选项。

约束：

- 类型字段必须有明确语义，不要用 `data`、`payload`、`extra` 承载未建模结构，除非确实是透传字段。
- 对外 DTO 不直接复用数据库 row，避免数据库结构泄漏到 API。
- 外部输入类型应尽量做反序列化边界处理，服务层继续做业务校验。
- 新增公开类型和字段时，应补充清晰注释，说明字段来源、含义和边界。

### 5.2 `service.rs`

`service.rs` 是业务用例层。它可以依赖 repository、client、config、state，但不应依赖具体 Web 框架响应类型。

适合放在 service 的逻辑：

- 权限判断和业务校验。
- 多个 repository 或外部 client 的调用编排。
- 事务边界。
- 输入归一化和业务默认值。
- 领域事件或副作用触发。

不应放在 service 的逻辑：

- Axum `Router`、`Json`、`StatusCode` 等 HTTP 细节。
- Tauri `Window`、`AppHandle` 等桌面壳细节，除非该模块明确就是桌面能力适配层。
- SQL 字符串拼接和数据库 row 映射。
- 前端展示文案和 UI 状态。

### 5.3 `repository.rs`

`repository.rs` 只负责持久化边界，包括 SQLite、文件系统、对象存储或本地缓存。

约束：

- repository 不做业务决策，只表达数据访问意图。
- SQL、迁移版本、表字段映射集中在 repository 或底层数据库 crate。
- 返回值应尽量是领域类型或明确的 row 类型，不返回松散 JSON。
- 数据库错误在此层转换为模块错误，避免上层到处匹配底层错误类型。

### 5.4 `routes.rs` 和 `commands.rs`

协议适配层只负责把外部协议转换为业务调用。

HTTP handler 推荐职责：

- 解析 path、query、body、state。
- 调用 service。
- 把 service result 转换为 HTTP response。

Tauri command 推荐职责：

- 解析前端参数。
- 获取应用状态。
- 调用 service。
- 把错误转换为前端稳定错误结构。

协议适配层不应直接访问数据库，也不应承载复杂业务规则。

### 5.5 `errors.rs`

每个业务模块可以拥有自己的错误类型，并在 crate 根部聚合为统一应用错误。

推荐：

```rust
pub type ProjectResult<T> = Result<T, ProjectError>;

#[derive(Debug, thiserror::Error)]
pub enum ProjectError {
    #[error("project not found: {project_id}")]
    NotFound { project_id: String },

    #[error("invalid project name")]
    InvalidName,

    #[error(transparent)]
    Database(#[from] sqlx::Error),
}
```

约束：

- 错误枚举要表达业务含义，不要只包装 `String`。
- 对外错误响应应稳定，避免直接泄漏底层错误文本。
- 边界层负责把模块错误转换为 HTTP status、Tauri error 或统一 API error。

## 6. 依赖方向

推荐依赖方向如下：

```text
main.rs
  -> app.rs
    -> state.rs / config.rs
    -> feature::routes 或 feature::commands
      -> feature::service
        -> feature::repository
        -> shared crate / external client
```

禁止或避免的方向：

- repository 反向依赖 service。
- service 依赖 routes 或 commands。
- 基础 crate 依赖应用 crate。
- 两个业务模块互相调用彼此内部文件。
- 为了复用少量函数引入跨业务的循环依赖。

如果两个业务模块需要协作，优先通过其中一个模块暴露的 service 门面调用；如果协作逻辑本身成为独立概念，再抽成新的业务模块。

## 7. 可见性规范

Rust 默认私有是优势，应主动利用。

推荐：

- 模块内部 helper 使用私有函数。
- 只在当前 crate 内共享的类型和函数使用 `pub(crate)`。
- 只有跨 crate 或外部调用确实需要的 API 才使用 `pub`。
- 不用 `pub mod` 暴露内部文件结构，优先在 `mod.rs` 中 `pub use`。
- 测试专用构造函数可以放在 `#[cfg(test)]` 模块内。

## 8. 命名规范

| 对象 | 命名 |
| --- | --- |
| crate | `kebab-case`，例如 `datavisool-project-format`。 |
| module/file | `snake_case`，例如 `ai_agent`、`runtime_database.rs`。 |
| struct/enum/trait | `PascalCase`，例如 `ProjectService`。 |
| function/method | `snake_case`，例如 `create_project`。 |
| constant | `SCREAMING_SNAKE_CASE`，例如 `DEFAULT_BIND_ADDR`。 |
| error enum | `{Domain}Error`，例如 `LicenseError`。 |
| result alias | `{Domain}Result<T>`，例如 `LicenseResult<T>`。 |

文件命名应表达业务语义。避免新增 `misc.rs`、`helper.rs`、`common.rs` 这类难以维护的文件名。

## 9. 应用入口组织

### 9.1 `main.rs`

`main.rs` 应保持很薄：

- 初始化日志。
- 读取配置。
- 构建应用状态。
- 启动 Tauri 或 HTTP server。
- 处理顶层错误。

业务逻辑、路由注册细节和具体初始化步骤应下沉到 `app.rs`、`state.rs` 或对应业务模块。

### 9.2 `app.rs`

`app.rs` 负责组装应用：

- 创建依赖对象。
- 执行迁移。
- 注册路由或命令。
- 返回可启动的 app/router/builder。

### 9.3 `state.rs`

`state.rs` 放置应用共享状态，例如连接池、配置、授权状态、取消句柄、运行时目录等。

约束：

- state 只承载共享依赖，不承载业务方法。
- 复杂业务操作应放到 service。
- 可变状态应明确并发语义，例如 `Arc`、`Mutex`、`RwLock` 的使用理由。

## 10. Tauri 后端建议结构

`datavisool-app/src-tauri` 适合采用“桌面壳 + 后端能力模块”的组织：

```text
src/
├── main.rs
├── app.rs
├── state.rs
├── db.rs
├── logging.rs
└── backend/
    ├── mod.rs
    ├── router.rs
    ├── error.rs
    ├── project/
    │   ├── mod.rs
    │   ├── types.rs
    │   ├── service.rs
    │   ├── repository.rs
    │   └── commands.rs
    ├── ai_model/
    │   ├── mod.rs
    │   ├── types.rs
    │   ├── service.rs
    │   ├── repository.rs
    │   └── commands.rs
    └── ai_driven/
        ├── mod.rs
        ├── model/
        └── model_invocation/
```

桌面端约束：

- `commands.rs` 作为 Tauri 命令入口，业务逻辑下沉到 `service.rs`。
- `router.rs` 或命令注册文件只做集中注册，不堆业务实现。
- `AppHandle`、窗口、通知、文件选择器等 Tauri 特有能力封装在 adapter 或 command 层。
- 可复用的纯 Rust 能力应抽到 `crates/*` 或新的共享 crate，而不是留在 `src-tauri`。

## 11. 测试组织

推荐三层测试：

```text
src/
└── project/
    ├── service.rs
    ├── repository.rs
    └── tests.rs
tests/
├── project_api.rs
└── migration.rs
```

测试规则：

- 纯函数、解析、校验、mapper 逻辑优先写模块内单元测试。
- service 层测试应覆盖成功路径、边界输入、权限或状态异常。
- repository 层测试应使用临时数据库或测试 fixture，不依赖开发者本机状态。
- HTTP/Tauri 外部行为放到 integration tests。
- 修复 bug 时优先补能复现问题的回归测试。

## 12. 迁移策略

整理现有 Rust 代码时应小步迁移，避免一次性大重构。

推荐步骤：

1. 先确定业务域清单，例如 `license`、`import`、`show`、`project`、`ai_model`、`ai_driven`。
2. 为目标业务域创建目录模块和 `mod.rs` 门面。
3. 先移动类型和错误，再移动 repository，最后移动 service 和 routes/commands。
4. 保持旧入口函数签名尽量不变，降低调用方同步修改范围。
5. 每迁移一个业务域就运行对应 crate 测试。
6. 删除只剩转发逻辑的旧文件，或在短期内保留兼容 re-export。
7. 迁移完成后再做命名、可见性和错误类型收敛。

不推荐：

- 一次性移动所有 Rust 文件。
- 在迁移同时重写业务逻辑。
- 同时调整数据库结构、API 协议和目录结构。
- 为了“看起来规整”拆出没有独立职责的小文件。

## 13. 评审检查清单

提交 Rust 代码前检查：

- 新代码是否放在正确业务模块内。
- `main.rs`、`app.rs` 是否保持启动和组装职责。
- service 是否没有依赖 HTTP/Tauri 响应类型。
- repository 是否没有业务决策。
- `mod.rs` 是否只导出必要 API。
- 新增公开类型、字段、方法和导出函数是否有清晰注释。
- 错误类型是否可诊断且不会泄漏敏感底层信息。
- 是否避免新增全局 `utils`、`common`、`types` 大杂烩。
- 测试是否覆盖新增业务分支或迁移风险点。
- 涉及 `crates` 时是否运行 `cargo test`。
- 涉及 `src-tauri` 时是否运行 `cargo test --manifest-path datavisool-app/src-tauri/Cargo.toml`。

## 14. 示例：新增业务模块

新增 `project` 模块时，优先按以下方式落地：

```text
src/project/
├── mod.rs
├── types.rs
├── service.rs
├── repository.rs
├── routes.rs
└── errors.rs
```

`mod.rs`：

```rust
mod errors;
mod repository;
mod routes;
mod service;
mod types;

pub use errors::{ProjectError, ProjectResult};
pub use routes::project_routes;
pub use service::ProjectService;
pub use types::{CreateProjectRequest, ProjectDetail, ProjectSummary};
```

调用方只依赖 `project::ProjectService`、`project::project_routes` 等门面 API，不直接引用 `project::repository::ProjectRepository`，除非它被明确设计为跨模块依赖。
