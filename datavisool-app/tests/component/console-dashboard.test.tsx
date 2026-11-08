/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import React from 'react';
import {fireEvent, render, screen, waitFor, within} from '@testing-library/react';
import {beforeEach, describe, expect, it, vi} from 'vitest';
import ConsoleDashboard from '@/pages/home/console/ConsoleDashboard.tsx';
import AuthFetchUtil from '@/util/AuthFetchUtil.ts';
import {formatDashboardTimestamp, getRecordTimestamp, parseDashboardTimestamp} from '@/pages/home/console/dashboardData.ts';

vi.mock('react-router-dom', () => ({useNavigate: () => vi.fn()}));
vi.mock('countup.js', () => ({CountUp: class {
    error = false;
    constructor(private element: HTMLElement, private value: number) {}
    start() { this.element.textContent = String(this.value); }
    reset() {}
}}));
vi.mock('@datavisool/ui', () => ({
    Button: ({children, onClick}: any) => <button onClick={onClick}>{children}</button>,
    globalMessage: {messageApi: {error: vi.fn()}},
}));
vi.mock('@/i18n/miniI18n.ts', () => ({tx: (text: string) => text}));
vi.mock('@/i18n/useLocaleVersion.ts', () => ({useLocaleVersion: () => 0}));
vi.mock('@/util/AuthFetchUtil.ts', () => ({default: {post: vi.fn()}}));
vi.mock('@/api/engineering/BaseApi.ts', () => ({default: {createProject: vi.fn()}}));
vi.mock('@/pages/home/project-list/NewProjectDialog.tsx', () => ({default: () => null}));

const post = vi.mocked(AuthFetchUtil.post);
const sqliteTime = (date: Date) => date.toISOString().replace('T', ' ').replace(/\.\d{3}Z$/, '');
const metric = (label: string) => screen.getByText(label).closest('article')!.querySelector('strong')!.textContent;

beforeEach(() => { post.mockReset(); });

describe('控制台数据', () => {
    it('按 UTC 解析 SQLite 时间，并保留带时区的时间', () => {
        expect(parseDashboardTimestamp('2026-11-06 04:28:09')).toBe(Date.parse('2026-11-06T04:28:09Z'));
        expect(parseDashboardTimestamp('2026-11-06 04:28:09.500')).toBe(Date.parse('2026-11-06T04:28:09.500Z'));
        expect(parseDashboardTimestamp('2026-11-06T04:28:09+08:00')).toBe(Date.parse('2026-11-06T04:28:09+08:00'));
        expect(parseDashboardTimestamp('invalid')).toBeNull();
    });

    it('统计全部分页中的近期项目和启用发布，并显示有效的创建时间', async () => {
        const time = sqliteTime(new Date(Date.now() - 60_000));
        const projects: {id: number; name: string; updateTime: string; createTime?: string}[] = Array.from({length: 51}, (_, id) => ({id, name: `项目${id}`, updateTime: time}));
        const releases: {id: number; name: string; state: number; updateTime: string; createTime?: string}[] = Array.from({length: 51}, (_, id) => ({
            id, name: `发布${id}`, state: id % 2 === 0 ? 1 : 0, updateTime: time,
        }));
        projects[0] = {id: 0, name: '回退项目', updateTime: 'invalid', createTime: time};
        releases[0] = {id: 0, name: '回退发布', state: 1, updateTime: 'invalid', createTime: time};
        post.mockImplementation(async (url: string, body: {current: number; size: number}) => {
            const records = url === '/api/project/pageList' ? projects : url === '/api/release/pageList' ? releases : [];
            const start = (body.current - 1) * body.size;
            return {total: records.length, records: records.slice(start, start + body.size)} as any;
        });
        render(<ConsoleDashboard/>);
        await screen.findByText('回退项目');
        await waitFor(() => expect(metric('近 2 周更新')).toBe('102'));
        expect(metric('项目总数')).toBe('51');
        expect(metric('已发布版本')).toBe('26');
        expect(post).toHaveBeenCalledWith('/api/project/pageList', {current: 2, size: 50});
        expect(post).toHaveBeenCalledWith('/api/release/pageList', {current: 2, size: 50});
        const row = screen.getByText('回退项目').closest('button')!;
        const expectedTime = formatDashboardTimestamp(getRecordTimestamp({createTime: time}));
        expect(within(row).getByText(expectedTime)).toBeInTheDocument();
        const releaseRow = screen.getByRole('cell', {name: '回退发布'}).closest('tr')!;
        expect(within(releaseRow).getByText(expectedTime)).toBeInTheDocument();
    });

    it('后续分页失败时显示提示，重试后恢复完整统计', async () => {
        let fail = true;
        const records = Array.from({length: 51}, (_, id) => ({id, updateTime: sqliteTime(new Date(Date.now() - 60_000))}));
        post.mockImplementation(async (url: string, body: {current: number; size: number}) => {
            if (url !== '/api/project/pageList') return {total: 0, records: []} as any;
            if (fail && body.current === 2) return null as any;
            return {total: 51, records: records.slice((body.current - 1) * body.size, body.current * body.size)} as any;
        });
        render(<ConsoleDashboard/>);
        expect(await screen.findByRole('alert')).toHaveTextContent('部分数据加载失败');
        expect(metric('近 2 周更新')).toBe('0');
        fail = false;
        fireEvent.click(screen.getByRole('button', {name: '重新加载'}));
        await waitFor(() => expect(screen.queryByRole('alert')).not.toBeInTheDocument());
        expect(metric('近 2 周更新')).toBe('51');
    });
});
