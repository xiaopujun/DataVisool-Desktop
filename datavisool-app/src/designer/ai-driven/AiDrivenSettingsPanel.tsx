/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
/*
 * Copyright © 2023-2025 puyinzhen
 * All rights reserved.
 */

import {Close, Refresh, Setting} from "@icon-park/react";
import {Button, globalMessage, NumberInput, Spin, Switch} from "@datavisool/ui";
import {useEffect, useState, type ReactNode} from "react";
import {
    getAiDrivenRuntimeSettings,
    saveAiDrivenRuntimeSettings,
    type IAiDrivenRuntimeSettings,
} from "@/api/AiDrivenSettingsApi.ts";
import {tx} from "@/i18n/miniI18n.ts";
import {useLocaleVersion} from "@/i18n/useLocaleVersion.ts";
import "./AiDrivenSettingsPanel.less";

interface AiDrivenSettingsPanelProps {
    visible: boolean;
    running: boolean;
    onClose: () => void;
}

interface SettingsFieldProps {
    fieldKey: string;
    label: string;
    description: string;
    value: number;
    min: number;
    max: number;
    step?: number;
    suffix: ReactNode;
    onChange: (value: number) => void;
}

const SettingsField = ({fieldKey, label, description, value, min, max, step, suffix, onChange}: SettingsFieldProps) => {
    const labelId = `ai-driven-setting-${fieldKey}`;
    return (
        <div className="ai-driven-settings-field" role="group" aria-labelledby={labelId}>
            <div className="ai-driven-settings-field-copy">
                <strong id={labelId}>{tx(label)}</strong>
                <span>{tx(description)}</span>
            </div>
            <NumberInput value={value}
                         min={min}
                         max={max}
                         step={step}
                         controls={false}
                         suffix={suffix}
                         onChange={(next) => {
                             if (typeof next === "number" && Number.isFinite(next))
                                 onChange(next);
                         }}/>
        </div>
    );
};

const AiDrivenSettingsPanel = ({visible, running, onClose}: AiDrivenSettingsPanelProps) => {
    useLocaleVersion();

    const [draft, setDraft] = useState<IAiDrivenRuntimeSettings | null>(null);
    const [defaults, setDefaults] = useState<IAiDrivenRuntimeSettings | null>(null);
    const [loading, setLoading] = useState(false);
    const [saving, setSaving] = useState(false);
    const [error, setError] = useState("");

    const loadSettings = () => {
        const controller = new AbortController();
        setLoading(true);
        setError("");
        getAiDrivenRuntimeSettings({signal: controller.signal, suppressErrorMessage: true})
            .then((response) => {
                setDraft(cloneSettings(response.settings));
                setDefaults(cloneSettings(response.defaults));
            })
            .catch((loadError) => {
                if (!controller.signal.aborted) {
                    console.error("load AI driven runtime settings failed", loadError);
                    setError(tx("读取 AI 运行设置失败，请重试"));
                }
            })
            .finally(() => {
                if (!controller.signal.aborted)
                    setLoading(false);
            });
        return () => controller.abort();
    };

    useEffect(() => {
        if (!visible)
            return;
        return loadSettings();
    }, [visible]);

    if (!visible)
        return null;

    const updateContext = <K extends keyof IAiDrivenRuntimeSettings["context"]>(key: K, value: number) => {
        setDraft((current) => current ? {...current, context: {...current.context, [key]: value}} : current);
    };
    const updateExecution = <K extends keyof IAiDrivenRuntimeSettings["execution"]>(key: K, value: number) => {
        setDraft((current) => current ? {...current, execution: {...current.execution, [key]: value}} : current);
    };
    const updateExecutionToggle = (
        key: "tokenBudgetEnabled" | "callBudgetEnabled" | "durationBudgetEnabled",
        value: boolean,
    ) => {
        setDraft((current) => current ? {...current, execution: {...current.execution, [key]: value}} : current);
    };
    const save = async () => {
        if (!draft)
            return;
        const validationError = validateSettings(draft);
        if (validationError) {
            setError(tx(validationError));
            return;
        }
        setSaving(true);
        setError("");
        try {
            const saved = await saveAiDrivenRuntimeSettings(draft, {suppressErrorMessage: true});
            setDraft(cloneSettings(saved));
            globalMessage.success(tx("AI 运行设置已保存"));
            onClose();
        } catch (saveError) {
            console.error("save AI driven runtime settings failed", saveError);
            setError(tx("保存失败，请检查配置后重试"));
        } finally {
            setSaving(false);
        }
    };

    return (
        <section className="ai-driven-settings-panel"
                 role="dialog"
                 aria-modal="false"
                 aria-labelledby="ai-driven-settings-title">
            <header className="ai-driven-settings-header">
                <div>
                    <span className="ai-driven-settings-icon" aria-hidden="true">
                        <Setting size={17} strokeWidth={3}/>
                    </span>
                    <div>
                        <strong id="ai-driven-settings-title">{tx("AI 运行设置")}</strong>
                        <span>{tx("控制新任务的上下文、超时与执行边界")}</span>
                    </div>
                </div>
                <Button className="ai-driven-settings-close"
                        type="text"
                        size="icon"
                        icon={<Close size={15}/>}
                        aria-label={tx("关闭 AI 运行设置")}
                        title={tx("关闭")}
                        onClick={onClose}/>
            </header>

            {loading ? (
                <div className="ai-driven-settings-loading">
                    <Spin size="large"/>
                    <span>{tx("正在读取设置…")}</span>
                </div>
            ) : error && !draft ? (
                <div className="ai-driven-settings-load-error" role="alert">
                    <span>{error}</span>
                    <Button type="default" size="sm" onClick={() => void loadSettings()}>{tx("重试")}</Button>
                </div>
            ) : draft ? (
                <>
                    <div className="ai-driven-settings-body">
                        {running && (
                            <div className="ai-driven-settings-notice" role="status">
                                {tx("当前任务继续使用启动时的配置；本次修改从下一个任务开始生效。")}
                            </div>
                        )}


                        <SettingsSection title="模型与上下文" description="控制单次模型连接与参与推理的上下文规模。">
                            <SettingsField fieldKey="request-timeout"
                                           label="模型请求超时"
                                           description="包括建立连接和完整读取流式响应的总时间。"
                                           value={draft.modelRequest.timeoutSeconds}
                                           min={30} max={3600} step={30} suffix={tx("秒")}
                                           onChange={(value) => setDraft({...draft, modelRequest: {timeoutSeconds: value}})}/>
                            <SettingsField fieldKey="active-context"
                                           label="活动上下文长度"
                                           description="模型目录窗口更小时，仍以模型实际窗口为准。"
                                           value={draft.context.maxActiveContextTokens}
                                           min={4096} max={2_000_000} step={1024} suffix="Token"
                                           onChange={(value) => updateContext("maxActiveContextTokens", value)}/>
                            <SettingsField fieldKey="tool-history-tokens"
                                           label="工具历史 Token 上限"
                                           description="用于模型历史投影的工具结果预算，与累计输入统计分开。"
                                           value={draft.context.maxToolHistoryTokens}
                                           min={2_048} max={1_000_000} step={512} suffix="Token"
                                           onChange={(value) => updateContext("maxToolHistoryTokens", value)}/>
                            <SettingsField fieldKey="state-digest-tokens"
                                           label="状态摘要 Token 上限"
                                           description="保留任务目标、回执和未完成工作的状态摘要上限。"
                                           value={draft.context.maxStateDigestTokens}
                                           min={512} max={32_768} step={512} suffix="Token"
                                           onChange={(value) => updateContext("maxStateDigestTokens", value)}/>
                            <SettingsField fieldKey="compact-percent"
                                           label="上下文压缩阈值"
                                           description="达到活动上下文的指定比例后开始压缩历史。"
                                           value={draft.context.compactAtPercent}
                                           min={50} max={95} suffix="%"
                                           onChange={(value) => updateContext("compactAtPercent", value)}/>
                        </SettingsSection>

                        <SettingsSection title="执行边界" description="限制任务总耗时和资源使用，避免长任务失控。">
                            <BudgetSwitchField fieldKey="token-budget-enabled"
                                                label="启用 Token 预算"
                                                description="新任务达到累计 Token 上限时停止并允许显式续跑。"
                                                value={draft.execution.tokenBudgetEnabled}
                                                onChange={(value) => updateExecutionToggle("tokenBudgetEnabled", value)}/>
                            <BudgetSwitchField fieldKey="call-budget-enabled"
                                                label="启用调用次数预算"
                                                description="新任务达到模型、工具或写入调用上限时停止。"
                                                value={draft.execution.callBudgetEnabled}
                                                onChange={(value) => updateExecutionToggle("callBudgetEnabled", value)}/>
                            <BudgetSwitchField fieldKey="duration-budget-enabled"
                                                label="启用时长预算"
                                                description="新任务达到任务时长上限时停止并保存进度。"
                                                value={draft.execution.durationBudgetEnabled}
                                                onChange={(value) => updateExecutionToggle("durationBudgetEnabled", value)}/>
                            <SettingsField fieldKey="task-duration"
                                           label="任务最长时间"
                                           description="达到上限后停止尚未完成的 Agent 任务。"
                                           value={draft.execution.maxDurationSeconds / 60}
                                           min={1} max={1440} suffix={tx("分钟")}
                                           onChange={(value) => updateExecution("maxDurationSeconds", value * 60)}/>
                            <SettingsField fieldKey="tool-duration"
                                           label="单次工具最长时间"
                                           description="单个工具调用不能超过任务最长时间。"
                                           value={draft.execution.maxToolDurationSeconds / 60}
                                           min={1} max={240} suffix={tx("分钟")}
                                           onChange={(value) => updateExecution("maxToolDurationSeconds", value * 60)}/>
                            <SettingsField fieldKey="total-tokens"
                                           label="累计 Token 上限"
                                           description="统计整个任务的模型输入和输出 Token。"
                                           value={draft.execution.maxTotalTokens}
                                           min={10_000} max={10_000_000} step={10_000} suffix="Token"
                                           onChange={(value) => updateExecution("maxTotalTokens", value)}/>
                        </SettingsSection>

                        <details className="ai-driven-settings-advanced">
                            <summary>
                                <span>{tx("高级限制")}</span>
                                <small>{tx("调用次数、压缩预算与失控保护")}</small>
                            </summary>
                            <div className="ai-driven-settings-advanced-content">
                                <SettingsField fieldKey="tool-result-tokens" label="工具结果 Token 上限"
                                               description="单项工具结果进入模型上下文前的最大长度。"
                                               value={draft.context.maxToolResultTokens}
                                               min={512} max={100_000} step={512} suffix="Token"
                                               onChange={(value) => updateContext("maxToolResultTokens", value)}/>
                                <SettingsField fieldKey="summary-tokens" label="压缩摘要 Token 上限"
                                               description="历史压缩后摘要可使用的最大长度。"
                                               value={draft.context.compactSummaryTokens}
                                               min={512} max={32_768} step={512} suffix="Token"
                                               onChange={(value) => updateContext("compactSummaryTokens", value)}/>
                                <SettingsField fieldKey="model-calls" label="模型调用次数"
                                               description="单个任务最多允许调用模型的次数。"
                                               value={draft.execution.maxModelCalls}
                                               min={1} max={1000} suffix={tx("次")}
                                               onChange={(value) => updateExecution("maxModelCalls", value)}/>
                                <SettingsField fieldKey="tool-calls" label="工具调用次数"
                                               description="单个任务全部工具调用的总上限。"
                                               value={draft.execution.maxToolCalls}
                                               min={1} max={5000} suffix={tx("次")}
                                               onChange={(value) => updateExecution("maxToolCalls", value)}/>
                                <SettingsField fieldKey="write-calls" label="写入调用次数"
                                               description="写入类工具的调用上限，不能超过工具调用总数。"
                                               value={draft.execution.maxWriteCalls}
                                               min={1} max={1000} suffix={tx("次")}
                                               onChange={(value) => updateExecution("maxWriteCalls", value)}/>
                                <SettingsField fieldKey="consecutive-failures" label="连续失败次数"
                                               description="工具连续失败达到上限后结束任务。"
                                               value={draft.execution.maxConsecutiveFailures}
                                               min={1} max={100} suffix={tx("次")}
                                               onChange={(value) => updateExecution("maxConsecutiveFailures", value)}/>
                                <SettingsField fieldKey="frontend-interactions" label="前端交互次数"
                                               description="允许 Agent 请求用户或画布交互的总次数。"
                                               value={draft.execution.maxFrontendInteractions}
                                               min={1} max={500} suffix={tx("次")}
                                               onChange={(value) => updateExecution("maxFrontendInteractions", value)}/>
                                <SettingsField fieldKey="same-read-repeats" label="相同读取重复次数"
                                               description="防止 Agent 对同一内容反复读取。"
                                               value={draft.execution.maxSameReadRepeats}
                                               min={1} max={20} suffix={tx("次")}
                                               onChange={(value) => updateExecution("maxSameReadRepeats", value)}/>
                                <SettingsField fieldKey="completion-continuations" label="完成续跑次数"
                                               description="完成门禁未满足时允许继续执行的次数。"
                                               value={draft.execution.maxCompletionContinuations}
                                               min={1} max={100} suffix={tx("次")}
                                               onChange={(value) => updateExecution("maxCompletionContinuations", value)}/>
                            </div>
                        </details>

                        {error && <div className="ai-driven-settings-error" role="alert">{error}</div>}
                    </div>

                    <footer className="ai-driven-settings-footer">
                        <span>{tx("设置自动保存到本地数据库")}</span>
                        <div>
                            <Button type="text"
                                    size="sm"
                                    icon={<Refresh size={14}/>}
                                    disabled={!defaults || saving}
                                    onClick={() => defaults && setDraft(cloneSettings(defaults))}>
                                {tx("恢复默认")}
                            </Button>
                            <Button type="primary" size="sm" loading={saving} onClick={() => void save()}>
                                {tx("保存设置")}
                            </Button>
                        </div>
                    </footer>
                </>
            ) : null}
        </section>
    );
};

const SettingsSection = ({title, description, children}: {title: string; description: string; children: ReactNode}) => (
    <section className="ai-driven-settings-section">
        <header>
            <strong>{tx(title)}</strong>
            <span>{tx(description)}</span>
        </header>
        <div>{children}</div>
    </section>
);

const BudgetSwitchField = ({
    fieldKey,
    label,
    description,
    value,
    onChange,
}: {
    fieldKey: string;
    label: string;
    description: string;
    value: boolean;
    onChange: (value: boolean) => void;
}) => {
    const labelId = `ai-driven-setting-${fieldKey}`;
    return (
        <div className="ai-driven-settings-field" role="group" aria-labelledby={labelId}>
            <div className="ai-driven-settings-field-copy">
                <strong id={labelId}>{tx(label)}</strong>
                <span>{tx(description)}</span>
            </div>
            <Switch value={value} onChange={onChange}/>
        </div>
    );
};

const cloneSettings = (settings: IAiDrivenRuntimeSettings): IAiDrivenRuntimeSettings => ({
    schemaVersion: settings.schemaVersion,
    modelRequest: {...settings.modelRequest},
    context: {...settings.context},
    execution: {...settings.execution},
});

const validateSettings = (settings: IAiDrivenRuntimeSettings): string => {
    if (settings.context.compactSummaryTokens > settings.context.maxActiveContextTokens)
        return "压缩摘要 Token 上限不能超过活动上下文长度";
    if (settings.context.maxToolResultTokens > settings.context.maxActiveContextTokens)
        return "工具结果 Token 上限不能超过活动上下文长度";
    if (settings.execution.maxWriteCalls > settings.execution.maxToolCalls)
        return "写入调用次数不能超过工具调用次数";
    if (settings.execution.durationBudgetEnabled && settings.execution.maxToolDurationSeconds > settings.execution.maxDurationSeconds)
        return "单次工具最长时间不能超过任务最长时间";
    return "";
};

export default AiDrivenSettingsPanel;
