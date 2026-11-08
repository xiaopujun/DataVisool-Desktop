# 桌面端自动化测试

在仓库根目录执行：

```sh
pnpm --filter datavisool-app test:desktop
```

该命令先构建包含 WebdriverIO 插件的 debug 程序，再运行 `tests/desktop` 下的测试。测试前端输出到 `.desktop-e2e/dist`，与普通构建分开。可在命令后传入 WebdriverIO 参数，例如 `--spec tests/desktop/smoke.spec.mjs`。

测试构建使用独立标识 `com.datavisool.desktop.e2e`，因此数据库和资源目录与日常应用分开。测试数据会保留供下次运行使用。自动化接口只用于测试构建，不应分发该构建；普通发布构建不启用 `desktop-e2e`。

WebDriver 操作应用的 WebView；系统原生对话框不在这些测试的覆盖范围内。

## 真实数据库回归（可选）

`database-live-regressions.spec.mjs` 默认跳过，需要通过环境变量 `DATAVISOOL_DB_TEST_CONFIG` 传入 JSON 数组。每项包含 `type`、`port`、`database`、`username`、`password`，SQL Server 可提供 `url`。数据库地址固定为 `127.0.0.1`。这些测试会创建表、函数及存储过程，应只使用临时数据库；测试后由运行者销毁数据库容器。MySQL 测试账号需要创建例程权限，并在开启二进制日志的测试实例上允许创建测试函数。

配置示例（替换为临时实例的端口和凭据）：

```sh
DATAVISOOL_DB_TEST_CONFIG='[{"type":"PostgresSQL","port":15432,"database":"test_db","username":"test_user","password":"test_password"}]' \
  pnpm --filter datavisool-app test:desktop --spec tests/desktop/database-live-regressions.spec.mjs
```
