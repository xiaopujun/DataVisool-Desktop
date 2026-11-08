-- Fix: replace UNIQUE index on ai_fusion_history(project_id, request_id)
-- with a plain index.  A single AI Fusion workflow execution creates
-- multiple history records (plan + tasks) that legitimately share the
-- same requestId.  Records are linked to assets through assetIds, not
-- through requestId uniqueness.
DROP INDEX IF EXISTS idx_ai_fusion_history_request;
CREATE INDEX IF NOT EXISTS idx_ai_fusion_history_request
  ON ai_fusion_history(project_id, request_id);
