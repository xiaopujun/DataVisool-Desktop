import assert from 'node:assert/strict';
import {constants, createCipheriv, createDecipheriv, generateKeyPairSync, privateDecrypt, publicEncrypt, randomBytes} from 'node:crypto';

const prefix = `数据库回归-${Date.now()}`;
const name = `${prefix}-连接`;
const secret = 'regression-secret';
let id;

async function request(url, method = 'POST', body = {current: 1, size: 100}) {
    return browser.execute(async (url, method, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method, body},
    }), url, method, body);
}

async function encryptedRequest(value) {
    const key = await request('/api/security/getEncryptKey', 'GET', null);
    assert.equal(key.code, 200);
    const aesKey = randomBytes(32);
    const iv = randomBytes(16);
    const cipher = createCipheriv('aes-256-cbc', aesKey, iv);
    const encryptedData = Buffer.concat([cipher.update(JSON.stringify(value), 'utf8'), cipher.final()]).toString('base64');
    const encryptedAesKey = publicEncrypt({key: key.data.key, padding: constants.RSA_PKCS1_PADDING}, Buffer.from(aesKey.toString('base64'))).toString('base64');
    return {encryptedData, encryptedAesKey, iv: iv.toString('base64')};
}

async function records() {
    const response = await request('/api/commonDatabase/pageList');
    assert.equal(response.code, 200);
    return response.data.records;
}

async function search(value) {
    await $('input[placeholder="搜索数据源名称/类型/描述"]').setValue(value);
    await browser.keys('Enter');
    await browser.pause(400);
}

async function focusTestWindow() {
    await browser.execute(async () => {
        if (document.visibilityState !== 'visible' || !document.hasFocus()) {
            await window.__TAURI__.window.getCurrentWindow().setFocus();
        }
    });
    await browser.waitUntil(async () => await browser.execute(() => (
        document.visibilityState === 'visible' && document.hasFocus()
    )), {timeout: 10000, interval: 100, timeoutMsg: 'Desktop test window did not regain focus'});
}

async function waitForEditorClosed() {
    await browser.waitUntil(async () => await browser.execute(() => (
        document.querySelector('.datasource-panel') === null
    )), {timeout: 5000, interval: 100});
}

async function openEditor(sourceName) {
    await focusTestWindow();
    const card = await $(`//article[contains(@class,"_dataSourceCard_")][.//h2[@title="${sourceName}"]]`);
    await card.$('./div[contains(@class,"cardActions")]/button[normalize-space()="编辑"]').click();
    await browser.waitUntil(() => browser.execute(expected => (
        document.querySelector('.datasource-panel input[placeholder="请输入数据库显示名称"]')?.value === expected
    ), sourceName), {timeout: 15000, timeoutMsg: 'Editor did not load the selected database'});
    // The window can lose focus during the details request. WebKit pauses animation frames while hidden.
    await browser.waitUntil(async () => {
        await focusTestWindow();
        return $('.datasource-panel.visible').isDisplayed();
    }, {timeout: 15000, timeoutMsg: 'Loaded database editor did not become visible'});
}

describe('database management regressions', function () {
    this.timeout(60000);
    before(async () => {
        await focusTestWindow();
        await $('//button[contains(@class,"datavisool-menu-submenu-trigger")][contains(.,"数据源管理")]').click();
        await $('.datavisool-app-menu-node-database button').click();
        await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/database'), {timeout: 10000});
        await browser.pause(800);
    });
    after(async () => {
        const items = (await records()).filter(item => item.name?.startsWith(prefix));
        if (items.length) {
            const response = await request('/api/commonDatabase/batchDel', 'POST', items.map(item => item.id));
            assert.equal(response.code, 200);
        }
        assert.equal((await records()).filter(item => item.name?.startsWith(prefix)).length, 0);
    });

    it('creates a source and searches by name, type and description without leaking its URL', async () => {
        await $('button[class*="_createButton_"]').click();
        await $('.datasource-panel [role="combobox"]').click();
        await $('//*[@role="option" and normalize-space()="MySQL"]').click();
        for (const [placeholder, value] of [
            ['请输入数据库显示名称', name], ['请输入 IP / 域名', '127.0.0.1'], ['请输入端口', '3306'],
            ['请输入用户名', 'regression-user'], ['请输入密码', secret], ['请输入数据库名称', 'sample'],
        ]) await $(`input[placeholder="${placeholder}"]`).setValue(value);
        await $('textarea[placeholder="请输入描述信息..."]').setValue(`${prefix}-独有描述`);
        await $('.datasource-panel .submit-btn').click();
        await browser.waitUntil(async () => (await records()).some(item => item.name === name), {timeout: 10000});
        const item = (await records()).find(record => record.name === name);
        id = item.id;
        assert.equal(Object.hasOwn(item, 'url'), false);
        assert.equal(JSON.stringify(item).includes(secret), false);
        await waitForEditorClosed();
        await openEditor(name);
        assert.equal(await $('.datasource-panel h2').getText(), '编辑数据库链接');
        const nameInput = await $('input[placeholder="请输入数据库显示名称"]');
        await nameInput.setValue('   ');
        assert.equal(await nameInput.getValue(), '   ');
        await $('.datasource-panel .submit-btn').click();
        assert.equal(await $('.datasource-panel .datasource-form-error').getText(), '请输入数据库显示名称');
        assert.equal((await records()).find(record => record.id === id).name, name);
        await $('.datasource-panel .cancel-btn').click();
        await waitForEditorClosed();
        for (const value of [name, 'MySQL', `${prefix}-独有描述`]) {
            await search(value);
            assert.ok((await browser.execute(() => document.querySelector('main').innerText)).includes(name), value);
        }
    });

    it('uses current connection fields and rejects invalid input in the editor', async () => {
        await search(name);
        await openEditor(name);
        const url = await $('input[placeholder="请输入 URL（填写 IP、端口、数据库名称后可不填）"]');
        assert.equal(await url.getValue(), '');
        assert.equal(await url.getAttribute('type'), 'password');
        const port = await $('input[placeholder="请输入端口"]');
        await port.setValue('65536');
        await $('.datasource-panel .submit-btn').click();
        assert.ok((await $('.datasource-panel').getText()).includes('端口必须在 1 至 65535 之间'));
        assert.equal((await records()).find(item => item.id === id).port, '3306');
        await port.setValue('3307');
        await $('input[placeholder="请输入密码"]').setValue('new-test-secret');
        await $('.datasource-panel .submit-btn').click();
        await browser.waitUntil(async () => (await records()).find(item => item.id === id)?.port === '3307', {timeout: 10000});
        await waitForEditorClosed();
        await search(name);
        await openEditor(name);
        assert.equal(await $('input[placeholder="请输入端口"]').getValue(), '3307');
        assert.equal(await $('input[placeholder="请输入密码"]').getValue(), 'new-test-secret');
        assert.equal(await $('input[placeholder="请输入 URL（填写 IP、端口、数据库名称后可不填）"]').getValue(), '');
        await $('.datasource-panel .cancel-btn').click();
        await waitForEditorClosed();
    });
    it('enforces validation for encrypted direct writes', async () => {
        const item = {name: `${prefix}-invalid`, type: 'MySQL', ip: '127.0.0.1', port: '3306', username: 'user', password: 'dummy', databaseName: 'sample'};
        for (const change of [{name: '   '}, {port: '0'}, {port: '65536'}, {des: 'a'.repeat(501)}]) {
            const response = await request('/api/commonDatabase/add', 'POST', await encryptedRequest({...item, ...change}));
            assert.equal(response.code, 400);
        }
        const oldPort = (await records()).find(item => item.id === id).port;
        const update = await request('/api/commonDatabase/update', 'POST', await encryptedRequest({...item, id, port: '65536'}));
        assert.equal(update.code, 400);
        assert.equal((await records()).find(item => item.id === id).port, oldPort);
    });

    it('preserves explicit custom URLs even when they match generated URLs', async () => {
        const base = {type: 'MySQL', ip: '127.0.0.1', port: '3306', username: 'user', password: 'dummy', databaseName: 'sample'};
        const generated = 'mysql://user:dummy@127.0.0.1:3306/sample';
        const keys = generateKeyPairSync('rsa', {modulusLength: 2048});
        const key = keys.publicKey.export({type: 'spki', format: 'der'}).toString('base64');
        async function read(id) {
            const response = await request('/api/commonDatabase/get', 'POST', {id, key});
            assert.equal(response.code, 200);
            const payload = response.data;
            const aesKey = Buffer.from(privateDecrypt({key: keys.privateKey, padding: constants.RSA_PKCS1_PADDING}, Buffer.from(payload.encryptedAesKey, 'base64')).toString(), 'base64');
            const decipher = createDecipheriv('aes-256-cbc', aesKey, Buffer.from(payload.iv, 'base64'));
            return JSON.parse(Buffer.concat([decipher.update(Buffer.from(payload.encryptedData, 'base64')), decipher.final()]).toString());
        }
        const old = await request('/api/commonDatabase/add', 'POST', await encryptedRequest({...base, name: `${prefix}-旧配置`, url: generated}));
        const custom = await request('/api/commonDatabase/add', 'POST', await encryptedRequest({...base, name: `${prefix}-自定义`, url: `${generated}?charset=utf8`}));
        assert.equal(old.code, 200);
        assert.equal(custom.code, 200);
        const oldConfig = await read(old.data);
        assert.equal(oldConfig.url, generated);
        oldConfig.port = '3307';
        assert.equal((await request('/api/commonDatabase/update', 'POST', await encryptedRequest(oldConfig))).code, 200);
        assert.equal((await read(old.data)).url, generated);
        const copy = await request(`/api/commonDatabase/copy/${old.data}`, 'GET', null);
        assert.equal(copy.code, 200);
        assert.equal((await read(copy.data)).url, generated);
        assert.equal((await read(custom.data)).url, `${generated}?charset=utf8`);
    });

});
