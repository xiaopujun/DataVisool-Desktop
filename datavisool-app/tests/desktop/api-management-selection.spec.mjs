import assert from 'node:assert/strict';

const name = `API 键盘选择 ${Date.now()}`;
let apiId;

async function request(url, method = 'POST', body) {
    return browser.execute(async (url, method, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method, body},
    }), url, method, body);
}

describe('API management card selection', function () {
    this.timeout(60000);

    before(async () => {
        await browser.switchToWindow('main');
        await browser.execute(() => window.__TAURI__.window.getCurrentWindow().setFocus());
        const response = await request('/api/commonApi/create', 'POST', {
            name,
            url: '/api/commonApi/pageList',
            method: 'GET',
            headers: '{}',
            params: '{}',
        });
        assert.equal(response.code, 200);
        apiId = response.data;

        if (await browser.execute(() => location.pathname !== '/home/api')) {
            const menuItem = await $('.datavisool-app-menu-node-api button');
            if (!(await menuItem.isDisplayed().catch(() => false))) {
                await $('//button[contains(@class,"datavisool-menu-submenu-trigger")][contains(.,"数据源管理")]').click();
            }
            await menuItem.click();
            await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/api'), {timeout: 10000});
        }
        await $('input[placeholder="搜索公共API"]').setValue(name);
        await browser.keys('Enter');
        await browser.waitUntil(async () => (await $('article[class*="_apiCard_"]').isDisplayed()), {timeout: 10000});
    });

    after(async () => {
        if (apiId) {
            const response = await request('/api/commonApi/batchDel', 'POST', [apiId]);
            assert.equal(response.code, 200);
        }
    });

    it('exposes a focusable native checkbox and preserves single-toggle card clicks', async () => {
        const checkbox = await $(`input[type="checkbox"][aria-label="选择 ${name}"]`);
        const card = await $('article[class*="_apiCard_"]');

        await checkbox.click();
        assert.equal(await checkbox.isSelected(), true);
        assert.ok((await card.getAttribute('class')).includes('_selectedCard_'));

        await checkbox.click();
        assert.equal(await checkbox.isSelected(), false);
        assert.ok(!(await card.getAttribute('class')).includes('_selectedCard_'));

        // The embedded driver's synthetic Tab does not run WebKit's native focus traversal.
        const control = await browser.execute(label => {
            const input = [...document.querySelectorAll('article input[type="checkbox"]')]
                .find(element => element.getAttribute('aria-label') === label);
            input.focus();
            return {tabIndex: input.tabIndex, focused: document.activeElement === input, disabled: input.disabled};
        }, `选择 ${name}`);
        assert.deepEqual(control, {tabIndex: 0, focused: true, disabled: false});
        await checkbox.click();
        await card.$('h2').click();
        assert.equal(await checkbox.isSelected(), false);
        assert.ok(!(await card.getAttribute('class')).includes('_selectedCard_'));
    });
});
