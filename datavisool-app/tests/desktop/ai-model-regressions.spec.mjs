import assert from 'node:assert/strict';

const prefix = `AI回归-${Date.now()}`;
const modelIds = [];
const apiIds = [];

async function request(url, method = 'POST', body = {current: 1, size: 100}) {
    return browser.execute(async (url, method, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method, body},
    }), url, method, body);
}

async function models() {
    const response = await request('/api/aiModel/pageList');
    assert.equal(response.code, 200);
    return response.data.records;
}

async function navigate(selector, path) {
    await $(selector).click();
    await browser.waitUntil(async () => await browser.execute((path) => location.pathname === path, path), {timeout: 10000});
    await browser.pause(700);
}

async function search(selector, value, expectedTotal) {
    await $(selector).setValue(value);
    await browser.keys('Enter');
    await browser.waitUntil(async () => (await $('.datavisool-pagination-total').getText()).includes(`共 ${expectedTotal} 条`), {timeout: 10000});
    await browser.pause(300);
}

async function modelMenu(action) {
    const button = await $('article[class*="_modelCard_"] button[aria-label="更多操作"]');
    await button.click();
    // The embedded driver click does not emit pointerdown, which opens Dropdown.
    await browser.keys('Enter');
    const item = await $(`//button[@role="menuitem"][.//span[text()="${action}"]]`);
    await item.waitForDisplayed({timeout: 5000});
    await item.click();
}

async function confirmDelete(action) {
    const dialog = await $('.datavisool-global-modal-card');
    await dialog.waitForDisplayed({timeout: 5000});
    assert.equal((await $$('.datavisool-global-modal-card')).length, 1, 'The confirmation must be rendered once');
    await $(`//section[contains(@class,"datavisool-global-modal-card")]//button[normalize-space()="${action}"]`).click();
}

describe('AI model and shared resource regressions', function () {
    this.timeout(60000);
    const matching = `${prefix}-匹配-`;
    const modelSearch = 'input[placeholder="搜索名称 / 提供商 / 接入地址"]';

    before(async () => {
        for (let index = 0; index < 14; index++) {
            const response = await request('/api/aiModel/create', 'POST', {
                name: index < 11 ? `${matching}${index}` : `${prefix}-其他-${index}`,
                provider: 'deepseek', baseUrl: 'https://regression.invalid/v1', apiKey: 'test-only-key', enabled: 1, isDefault: 0,
            });
            assert.equal(response.code, 200);
            modelIds.push(response.data);
        }
        await navigate('.datavisool-app-menu-node-aiModel button', '/home/aiModel');
    });

    after(async () => {
        if (modelIds.length) {
            const response = await request('/api/aiModel/batchDel', 'POST', modelIds);
            assert.equal(response.code, 200);
        }
        if (apiIds.length) {
            const response = await request('/api/commonApi/batchDel', 'POST', apiIds);
            assert.equal(response.code, 200);
        }
    });

    it('clears a saved API key while keeping new-model validation', async () => {
        const name = `${matching}0`;
        await search(modelSearch, name, 1);
        await $('button[class*="_editAction_"]').click();
        const clear = await $('//div[@data-ai-model-field][.//*[contains(text(),"API Key 操作")]]//button[@role="switch"]');
        await clear.waitForDisplayed({timeout: 5000});
        await clear.click();
        assert.equal(await $('input[type="password"]').isEnabled(), false);
        await $('[data-ai-model-action="submit"]').click();
        await browser.waitUntil(async () => (await models()).find(item => item.id === modelIds[0])?.hasApiKey === false, {timeout: 10000});
        await browser.waitUntil(async () => !(await $('[data-ai-model-title]').isDisplayed()), {timeout: 5000});

        await $('button[class*="_createButton_"]').click();
        await $('input[placeholder="例如：默认对话模型"]').setValue(`${prefix}-无密钥`);
        await $('[data-ai-model-action="submit"]').click();
        assert.ok((await $('[role="dialog"]').getText()).includes('请输入API Key'));
        await $('[data-ai-model-action="cancel"]').click();
    });

    it('displays UTC database timestamps as local time', async () => {
        const item = (await models()).find(record => record.id === modelIds[0]);
        await search(modelSearch, item.name, 1);
        const expected = await browser.execute((stamp) => {
            const date = new Date(stamp.replace(' ', 'T') + 'Z');
            const pad = value => String(value).padStart(2, '0');
            return `${pad(date.getMonth() + 1)}/${pad(date.getDate())} ${pad(date.getHours())}:${pad(date.getMinutes())}`;
        }, item.updateTime);
        assert.ok((await $('[class*="_footerTime_"]').getText()).includes(expected));
    });

    it('preserves the filter on page changes and returns to the last valid page after deletion', async () => {
        await search(modelSearch, matching, 11);
        await $('[aria-label="第 2 页"]').click();
        await browser.waitUntil(async () => (await $$('article[class*="_modelCard_"]')).length === 1, {timeout: 10000});
        assert.ok((await $('.datavisool-pagination-total').getText()).includes('共 11 条'));
        assert.ok((await $('article[class*="_modelCard_"]').getText()).includes(matching));
        await modelMenu('删除');
        await confirmDelete('取消');
        await modelMenu('删除');
        await confirmDelete('确定');
        await browser.waitUntil(async () => (await $$('article[class*="_modelCard_"]')).length === 10, {timeout: 10000});
        assert.ok((await $('.datavisool-pagination-total').getText()).includes('共 10 条'));
        assert.equal(await $('.datavisool-pagination-item-active').getText(), '1');
        const names = await browser.execute(() => [...document.querySelectorAll('article[class*="_modelCard_"] [class*="_cardTitle_"]')].map(item => item.textContent));
        assert.ok(names.every(name => name.includes(matching)));
    });

    it('preserves the same pagination behavior on the shared API management page', async () => {
        for (let index = 0; index < 13; index++) {
            const response = await request('/api/commonApi/create', 'POST', {
                name: index < 11 ? `${matching}API-${index}` : `${prefix}-其他API-${index}`,
                url: 'https://regression.invalid/api', method: 'GET',
            });
            assert.equal(response.code, 200);
            apiIds.push(response.data);
        }
        await $('//button[contains(@class,"datavisool-menu-submenu-trigger")][contains(.,"数据源管理")]').click();
        await navigate('.datavisool-app-menu-node-api button', '/home/api');
        await search('input[placeholder="搜索公共API"]', `${matching}API-`, 11);
        await $('[aria-label="第 2 页"]').click();
        await browser.waitUntil(async () => (await $$('article[class*="_apiCard_"]')).length === 1, {timeout: 10000});
        assert.ok((await $('.datavisool-pagination-total').getText()).includes('共 11 条'));
        await $('//article[contains(@class,"_apiCard_")]//button[contains(.,"删除")]').click();
        await confirmDelete('确定');
        await browser.waitUntil(async () => (await $$('article[class*="_apiCard_"]')).length === 10, {timeout: 10000});
        assert.ok((await $('.datavisool-pagination-total').getText()).includes('共 10 条'));
        assert.equal(await $('.datavisool-pagination-item-active').getText(), '1');
    });
});
