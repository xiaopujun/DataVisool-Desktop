import assert from 'node:assert/strict';

const fixturePrefix = `控制台回归-${Date.now()}`;
const projectIds = [];
const releaseIds = [];

async function request(url, method = 'POST', body = {current: 1, size: 50}) {
    return browser.execute(async (url, method, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method, body},
    }), url, method, body);
}

async function openConsole() {
    await $('.datavisool-app-menu-node-server button').click();
    await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/server'), {timeout: 10000});
    await $('.datavisool-app-menu-node-console button').click();
    await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/console'
        && document.querySelector('[aria-busy]')?.getAttribute('aria-busy') === 'false'), {timeout: 30000});
    await browser.pause(1000);
}

describe('console dashboard regressions', function () {
    before(async () => {
        await $('.datavisool-app-menu-node-console button').waitForDisplayed({timeout: 30000});
        for (let index = 0; index < 51; index++) {
            const response = await request('/api/project/create', 'POST', {
                name: `${fixturePrefix}-${index}`, des: '',
                dataJson: JSON.stringify({canvasManager: {width: 1920, height: 1080}}),
            });
            assert.equal(response.code, 200);
            projectIds.push(response.data);
        }
        for (let index = 0; index < 2; index++) {
            const response = await request('/api/release/release', 'POST', {
                releaseType: 'NEW', projectId: String(projectIds[index]), encrypt: false, dependencies: [],
            });
            assert.equal(response.code, 200);
            releaseIds.push(response.data);
        }
        const disabled = await request('/api/release/update', 'POST', {id: releaseIds[1], state: 0});
        assert.equal(disabled.code, 200);
    });

    after(async () => {
        if (releaseIds.length) {
            const response = await request('/api/release/batchDel', 'POST', releaseIds);
            assert.equal(response.code, 200);
        }
        for (const id of projectIds) {
            const response = await request(`/api/project/del/${id}`, 'GET', null);
            assert.equal(response.code, 200);
        }
    });

    it('loads every page, counts enabled releases, and shows local time and complete summaries', async () => {
        const [projects, releases] = await Promise.all([
            request('/api/project/pageList', 'POST', {current: 1, size: 10000}),
            request('/api/release/pageList', 'POST', {current: 1, size: 10000}),
        ]);
        assert.equal(projects.code, 200);
        assert.equal(releases.code, 200);
        await openConsole();
        await browser.waitUntil(async () => await browser.execute((expected) => {
            const value = document.querySelector('section[aria-label="核心指标"] article strong')?.textContent;
            return Number(value?.replaceAll(',', '')) === expected;
        }, projects.data.total), {timeout: 15000, timeoutMsg: 'Project count animation did not finish'});
        const state = await browser.execute((prefix) => {
            const cards = [...document.querySelectorAll('section[aria-label="核心指标"] article')];
            const metrics = Object.fromEntries(cards.map((card) => [card.querySelector('h2')?.textContent,
                Number(card.querySelector('strong')?.textContent?.replaceAll(',', ''))]));
            const summaries = [...document.querySelectorAll('[class*="kpiTrendSummary_"]')]
                .map((element) => ({text: element.textContent, width: element.clientWidth, scrollWidth: element.scrollWidth, height: element.clientHeight, lineHeight: parseFloat(getComputedStyle(element).lineHeight)}));
            const item = [...document.querySelectorAll('button[data-type="project"]')].find((element) => element.textContent?.includes(prefix));
            const time = item?.querySelector('time')?.textContent;
            const parts = time?.match(/^(\d+)\/(\d+)\/(\d+) (\d+):(\d+)$/);
            const displayedTime = parts && new Date(Number(parts[1]), Number(parts[2]) - 1, Number(parts[3]), Number(parts[4]), Number(parts[5])).getTime();
            const ageMs = displayedTime == null ? null : Date.now() - displayedTime;
            return {metrics, summaries, time, ageMs, alert: document.querySelector('[role="alert"]')?.textContent};
        }, fixturePrefix);
        console.log('CONSOLE_REGRESSION', JSON.stringify(state));
        assert.ok(!state.alert);
        assert.equal(state.metrics['项目总数'], projects.data.total);
        const now = new Date();
        const start = new Date(now.getFullYear(), now.getMonth(), now.getDate()).getTime() - 13 * 86400000;
        const recentProjects = projects.data.records.filter((record) => {
            const value = record.updateTime || record.createTime;
            const timestamp = Date.parse(/^\d{4}-\d{2}-\d{2} /.test(value) ? `${value.replace(' ', 'T')}Z` : value);
            return timestamp >= start && timestamp <= now.getTime();
        }).length;
        assert.ok(recentProjects >= 51);
        assert.equal(state.summaries[0].text, `近 2 周 · ${recentProjects} 项更新`);
        assert.ok(state.metrics['近 2 周更新'] >= 53);
        assert.equal(state.metrics['已发布版本'], releases.data.records.filter((record) => record.state === true || record.state === 1).length);
        assert.ok(state.ageMs !== null && state.ageMs >= 0 && state.ageMs < 120000, `Expected recent local time, got ${state.time}`);
        assert.ok(state.summaries.every((summary) => summary.scrollWidth <= summary.width
            && summary.height <= summary.lineHeight + 1), JSON.stringify(state.summaries));
    });

    it('opens and centers the project dialog even when animation frame callbacks do not run', async () => {
        await browser.execute(() => {
            window.__originalAnimationFrame = window.requestAnimationFrame;
            window.requestAnimationFrame = () => 1;
        });
        try {
            await $('button[class*="newProjectButton"]').click();
            const dialog = await $('.new-project-dialog.visible');
            await dialog.waitForDisplayed({timeout: 5000});
            await browser.pause(250);
            const position = await browser.execute(() => {
                const rect = document.querySelector('.new-project-dialog').getBoundingClientRect();
                return {x: rect.x, y: rect.y, width: rect.width, height: rect.height, windowWidth: innerWidth, windowHeight: innerHeight};
            });
            assert.ok(Math.abs(position.x - Math.max(16, (position.windowWidth - position.width) / 2)) < 4, JSON.stringify(position));
            assert.ok(Math.abs(position.y - Math.max(16, (position.windowHeight - position.height) / 2)) < 4, JSON.stringify(position));
            await $('.new-project-dialog .btn-cancel').click();
        } finally {
            await browser.execute(() => { window.requestAnimationFrame = window.__originalAnimationFrame; });
        }
    });

});
