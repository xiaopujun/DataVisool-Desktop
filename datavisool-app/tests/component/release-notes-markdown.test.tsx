/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {cleanup, render, screen} from "@testing-library/react";
import {afterEach, describe, expect, test} from "vitest";
import ReleaseNotesMarkdown from "@/comps/common-component/update-release-notes/ReleaseNotesMarkdown.tsx";

describe("ReleaseNotesMarkdown", () => {
    afterEach(cleanup);

    test("renders release note Markdown as semantic elements", () => {
        render(
            <ReleaseNotesMarkdown
                content={[
                    "# DataVisool 2026.8.0",
                    "",
                    "## 新增",
                    "",
                    "- 支持 **自定义快捷键**",
                    "- 优化 `ScreenFit` 适配",
                    "",
                    "| 调整项 | 变更内容 |",
                    "|---|---|",
                    "| **配色方案** | 银灰色渐变 |",
                    "",
                    "> 建议升级到此版本",
                    "",
                    "```ts",
                    "const version = \"2026.8.0\";",
                    "```",
                ].join("\n")}
                fallback="是否立即更新？"
            />
        );

        expect(screen.getByRole("heading", {level: 1, name: "DataVisool 2026.8.0"})).toBeInTheDocument();
        expect(screen.getByRole("heading", {level: 2, name: "新增"})).toBeInTheDocument();
        expect(screen.getByRole("list")).toHaveTextContent("支持 自定义快捷键");
        expect(screen.getByText("自定义快捷键").tagName).toBe("STRONG");
        expect(screen.getByText("ScreenFit").tagName).toBe("CODE");
        expect(screen.getByRole("table")).toHaveTextContent("调整项变更内容配色方案银灰色渐变");
        expect(screen.getByText("建议升级到此版本").closest("blockquote")).toBeInTheDocument();
        expect(screen.getByText('const version = "2026.8.0";').closest("pre")).toBeInTheDocument();
    });

    test("renders unsafe HTML as text instead of injecting elements", () => {
        const {container} = render(
            <ReleaseNotesMarkdown content="<script>window.alert('unsafe')</script>" fallback="是否立即更新？"/>
        );

        expect(container.querySelector("script")).not.toBeInTheDocument();
        expect(container).toHaveTextContent("<script>window.alert('unsafe')</script>");
    });

    test("keeps the active streaming tail lightweight", () => {
        render(
            <ReleaseNotesMarkdown content={"**已完成段落**\n\n正在生成 `代码`"}
                                  fallback="等待内容"
                                  streaming/>
        );

        expect(screen.getByText("已完成段落").tagName).toBe("STRONG");
        expect(screen.getByText("代码").tagName).toBe("CODE");
        expect(document.querySelector("[aria-hidden='true']")).toBeInTheDocument();
    });

    test("renders fallback content when release notes are empty", () => {
        render(<ReleaseNotesMarkdown content="  " fallback="是否立即更新？"/>);

        expect(screen.getByText("是否立即更新？")).toBeInTheDocument();
    });
});
