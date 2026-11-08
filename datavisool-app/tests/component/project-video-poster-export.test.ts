/*
 * Copyright © 2023–2026 xiaopujun. All rights reserved.
 * Licensed under the DataVisool License. See the LICENSE file distributed with this software.
 */
import {beforeEach, describe, expect, it} from 'vitest';
import ProjectUtil from '@/util/ProjectUtil.ts';

const project = (elements: Record<string, unknown>) => ({layerManager: {elemConfigs: elements, layerConfigs: {}, layerHeader: []}} as any);
const video = (poster: string) => ({base: {type: 'BaseVideo'}, style: {poster, src: '/static/videos/video.mp4'}});

describe('video poster export dependencies', () => {
    beforeEach(() => { window.DATAVISOOL_ENV = {definitions: {}, remoteScripts: {}} as any; });

    it('collects local posters in nested loops, decodes names, strips URL suffixes and deduplicates', () => {
        const data = project({
            first: video('asset://localhost/C%3A/project/static/images/my%20poster%2B.png?size=1#top'),
            loop: {base: {type: 'LoopContainer'}, managerData: project({nested: video('/static/images/my%20poster%2B.png')})},
            external: video('https://example.com/static/images/not-local.png'),
            data: video('data:image/png;base64,abc'),
        });
        const result = ProjectUtil.calculateProjectDependencyByProject('1', 'test', data);
        expect(result.images).toEqual(['my poster+.png']);
        expect(result.videos).toEqual(['video.mp4']);
    });

    it('includes posters from referenced projects in the project graph', async () => {
        const entry = project({reference: {base: {type: 'ScreenReference'}, style: {screenId: '2'}}, first: video('/static/images/shared.png')});
        const child = project({nested: video('http://asset.localhost/tmp/static/images/shared.png'), other: video('/static/images/child.png')});
        const metas: Record<string, any> = {'1': {id: '1', name: 'entry', data: entry}, '2': {id: '2', name: 'child', data: child}};
        const graph = await ProjectUtil.calculateProjectGraphDependency('1', entry, {loadProjectMeta: async id => metas[id]});
        expect(graph.images.sort()).toEqual(['child.png', 'shared.png']);
        expect(graph.projects.find(item => item.id === '2')?.images.sort()).toEqual(['child.png', 'shared.png']);
    });
});
