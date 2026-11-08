-- AI 模型管理页只保留厂商接入信息，不再在 ai_model 表里存储固定模型型号。
-- SQLite 3.35+ 支持 DROP COLUMN，可直接移除 model_name。
ALTER TABLE ai_model DROP COLUMN model_name;
