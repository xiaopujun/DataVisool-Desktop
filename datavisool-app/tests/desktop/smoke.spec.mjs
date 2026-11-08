import assert from 'node:assert/strict';

describe('Tauri desktop application', () => {
    it('opens settings and reads the real Rust backend configuration', async () => {
        const settings = await $('.datavisool-app-menu-node-setting button');
        await settings.waitForDisplayed({timeout: 30000});
        await settings.click();
        await browser.waitUntil(async () => await browser.execute(() => window.location.pathname === '/home/setting'), {
            timeout: 10000,
            timeoutMsg: 'The settings page did not open',
        });
        assert.equal(await browser.tauri.execute(() => window.location.pathname), '/home/setting');

        const response = await browser.execute(async () => {
            return window.__TAURI__.core.invoke('datavisool_http_request', {
                req: {url: '/api/appConfig/get', method: 'GET'},
            });
        });
        assert.equal(response.code, 200);
        assert.ok(Object.hasOwn(response.data, 'resourceStorageDir'));
    });
});
