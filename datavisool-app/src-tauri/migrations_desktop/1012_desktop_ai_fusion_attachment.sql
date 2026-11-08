-- AI 控制台上传附件表
--
-- SQLite 不支持 MySQL 风格的 COMMENT 语法，
-- 因此这里使用标准 SQL 注释说明表和字段语义。
--
-- 设计目标：
-- 1. 承载“先上传图片，再在 AI 请求里引用 attachmentId”的 V2 方案
-- 2. 与 ai_fusion_history / ai_fusion_history_asset 解耦，避免把临时上传态和历史结果态混在一起
-- 3. 支持会话复用、过期清理、按请求排查上传链路

CREATE TABLE IF NOT EXISTS ai_fusion_attachment (
  -- 附件主键，同时作为前端后续引用的 attachmentId
  id TEXT PRIMARY KEY,

  -- 归属项目 id，限制附件作用域，避免跨项目误引用
  project_id INTEGER NOT NULL,

  -- AI 控制台会话 id。
  -- 一个会话里可能会多次发送 AI 请求并复用同一张图片，因此不能只靠 requestId 标识生命周期
  session_id TEXT NOT NULL,

  -- 上传这张图片时对应的 requestId，用于排查上传链路、取消上传和审计上传动作
  upload_request_id TEXT NOT NULL,

  -- 文件 MIME 类型，例如 image/png
  mime_type TEXT NOT NULL,

  -- 原始文件名
  original_name TEXT NOT NULL,

  -- 文件内容哈希，用于去重和完整性校验
  sha256 TEXT NOT NULL,

  -- 落盘后的相对路径，避免把 dataUrl/base64 等传输层临时数据直接存库
  relative_path TEXT NOT NULL,

  -- 原始字节大小
  byte_size INTEGER NOT NULL,

  -- 图片宽高，无法识别时允许为空
  width INTEGER,
  height INTEGER,

  -- 附件状态。
  -- 当前建议值：
  -- ready   : 可被 AI 请求正常引用
  -- expired : 已过期，等待或已完成清理
  -- deleted : 已逻辑删除
  status TEXT NOT NULL,

  -- 过期时间戳（毫秒）
  expire_at INTEGER NOT NULL,

  -- 最近一次被 AI 请求引用的时间戳（毫秒）
  last_used_at INTEGER,

  -- 创建/更新时间戳（毫秒）
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,

  -- 逻辑删除标记
  deleted INTEGER NOT NULL DEFAULT 0,

  FOREIGN KEY(project_id) REFERENCES project(id)
);

-- 按项目 + 会话列出附件，供控制台会话恢复和清理使用
CREATE INDEX IF NOT EXISTS idx_ai_fusion_attachment_project_session
  ON ai_fusion_attachment(project_id, session_id, deleted, created_at DESC);

-- 通过上传 requestId 追踪一次上传动作产生的所有附件
CREATE INDEX IF NOT EXISTS idx_ai_fusion_attachment_upload_request
  ON ai_fusion_attachment(upload_request_id);

-- 按项目 + 哈希检索，支持后续做同项目去重或秒传
CREATE INDEX IF NOT EXISTS idx_ai_fusion_attachment_project_hash
  ON ai_fusion_attachment(project_id, sha256, deleted);

-- 按过期时间和状态扫描，供后台清理任务使用
CREATE INDEX IF NOT EXISTS idx_ai_fusion_attachment_expire
  ON ai_fusion_attachment(expire_at, deleted, status);
