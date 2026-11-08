import assert from 'node:assert/strict';
import {existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync} from 'node:fs';
import {join} from 'node:path';

const localeKey = 'datavisool-i18n-locale';
const version = JSON.parse(readFileSync(new URL('../../package.json', import.meta.url), 'utf8')).version;
const fontName = `设置回归-${Date.now()}`;
let root;
let originalConfig;
let originalLocale;
let fontIds = [];

async function request(url, method = 'GET', body) {
    return browser.execute(async (url, method, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method, body},
    }), url, method, body);
}

async function focus() {
    await browser.execute(async () => {
        if (document.visibilityState !== 'visible' || !document.hasFocus())
            await window.__TAURI__.window.getCurrentWindow().setFocus();
    });
    await browser.waitUntil(async () => await browser.execute(() => document.visibilityState === 'visible' && document.hasFocus()),
        {timeout: 10000});
}

async function menu(label) {
    await $(`//button[@role="menuitem" and normalize-space()="${label}"]`).click();
}

async function chooseLanguage(label) {
    await $('[role="combobox"]').click();
    await $(`//*[@role="option" and normalize-space()="${label}"]`).click();
}

async function waitPath(value) {
    await browser.waitUntil(async () => await browser.execute(path => document.querySelector('input[readonly]')?.value === path, value),
        {timeout: 10000});
}

describe('设置和关于页面回归', function () {
    this.timeout(120000);

    before(async () => {
        root = mkdtempSync('/private/tmp/datavisool-settings-regression-');
        writeFileSync(join(root, 'not-a-directory'), 'test');
        await $('.datavisool-app-menu-node-setting button').waitForDisplayed({timeout: 30000});
        await $('.datavisool-app-menu-node-setting button').click();
        await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/setting'
            && !!document.querySelector('[role="combobox"]')), {timeout: 30000});
        await focus();
        originalConfig = (await request('/api/appConfig/get')).data.resourceStorageDir;
        originalLocale = await browser.execute(key => localStorage.getItem(key), localeKey);
        await browser.execute(() => window.datavisoolI18n.setLocale('zh-CN'));
        await browser.waitUntil(async () => await browser.execute(() => document.body.innerText.includes('资源存储位置')),
            {timeout: 5000});
    });

    after(async () => {
        await browser.execute(() => { if (window.__settingsOriginalDate) window.Date = window.__settingsOriginalDate; });
        if (fontIds.length) await request('/api/font/batchDel', 'POST', fontIds);
        if (originalConfig !== undefined) {
            const restored = await request('/api/appConfig/save', 'POST', {resourceStorageDir: originalConfig});
            assert.equal(restored.code, 200);
        }
        await browser.execute(({key, value}) => {
            if (value === null) localStorage.removeItem(key);
            else localStorage.setItem(key, value);
        }, {key: localeKey, value: originalLocale});
        if (root) rmSync(root, {recursive: true, force: true});
    });

    it('中英文切换与刷新后保持', async () => {
        await chooseLanguage('English');
        await browser.waitUntil(async () => await browser.execute(key => localStorage.getItem(key) === 'en-US', localeKey),
            {timeout: 5000});
        await menu('About');
        await browser.waitUntil(async () => await browser.execute(() => !!document.querySelector('[class*="aboutTopVersion"]')),
            {timeout: 5000});
        assert.ok((await browser.execute(() => document.body.innerText)).includes('Version Information'));
        await menu('General');
        assert.equal(await $('[role="combobox"]').getText(), 'English');
        await browser.refresh();
        await browser.waitUntil(async () => await browser.execute(() => !!document.querySelector('[role="combobox"]')),
            {timeout: 30000});
        await focus();
        assert.equal(await $('[role="combobox"]').getText(), 'English');
        await chooseLanguage('Simplified Chinese');
        await browser.waitUntil(async () => await browser.execute(key => localStorage.getItem(key) === 'zh-CN', localeKey),
            {timeout: 5000});
    });

    it('读取目录、写入资源、恢复默认和目录校验', async () => {
        assert.equal((await request('/api/appConfig/save', 'POST', {resourceStorageDir: root})).code, 200);
        await menu('关于');
        await menu('通用');
        await waitPath(root);
        const uploaded = await request('/api/font/upload', 'POST', {fonts: [{
            fileName: `${fontName}.ttf`, dataUrl: `data:font/ttf;base64,${Buffer.from(fontName).toString('base64')}`,
        }]});
        assert.equal(uploaded.code, 200);
        fontIds = uploaded.data;
        const fonts = await request('/api/font/pageList', 'POST', {current: 1, size: 100, searchValue: fontName});
        const font = fonts.data.records.find(item => item.name === fontName);
        assert.ok(font.url.includes(encodeURIComponent(root)));
        assert.ok(existsSync(decodeURIComponent(new URL(font.url).pathname)));
        await $('[aria-label="清空并恢复默认"]').click();
        await waitPath('');
        assert.equal((await request('/api/appConfig/get')).data.resourceStorageDir, null);
        assert.equal((await request('/api/appConfig/save', 'POST', {resourceStorageDir: 'relative/path'})).code, 400);
        assert.notEqual((await request('/api/appConfig/save', 'POST', {resourceStorageDir: join(root, 'not-a-directory')})).code, 200);
        assert.equal((await request('/api/appConfig/get')).data.resourceStorageDir, null);
    });

    it('关于页面显示版本且不显示未经核实的日期', async () => {
        await menu('关于');
        await browser.waitUntil(async () => await browser.execute(() => !!document.querySelector('[class*="aboutTopVersion"]')),
            {timeout: 5000});
        const about = await browser.execute(() => ({
            version: document.querySelector('[class*="aboutTopVersion"]')?.textContent,
            text: document.body.innerText,
            images: [...document.querySelectorAll('img')].map(image => image.naturalWidth),
            viewport: innerWidth,
            scrollWidth: document.documentElement.scrollWidth,
        }));
        assert.ok(about.version.includes(version));
        assert.ok(about.images.every(width => width > 0));
        assert.ok(about.scrollWidth <= about.viewport);
        assert.ok(!about.text.includes('发布日期'));
        await menu('通用');
        await browser.execute(() => {
            window.__settingsOriginalDate = Date;
            const OriginalDate = Date;
            window.Date = class extends OriginalDate {
                constructor(...args) { super(...(args.length ? args : [OriginalDate.now() + 3 * 86400000])); }
                static now() { return OriginalDate.now() + 3 * 86400000; }
            };
        });
        await menu('关于');
        assert.ok(!(await browser.execute(() => document.body.innerText)).includes('发布日期'));
        await browser.execute(() => { window.Date = window.__settingsOriginalDate; });
    });
});
