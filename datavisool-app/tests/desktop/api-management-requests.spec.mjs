import assert from 'node:assert/strict';
import {createServer} from 'node:http';

const dialog = '.datavisool-dialog';
const address = 'input[placeholder="请输入完整的请求 URL"]';
const testName = `API直连回归-${Date.now()}`;
let server;
let origin;

async function api(url, body) {
    return browser.execute((url, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method: 'POST', body},
    }), url, body);
}

async function matchingRecords() {
    const result = await api('/api/commonApi/pageList', {current: 1, size: 10, searchValue: testName});
    assert.equal(result.code, 200);
    return result.data.records.filter(row => row.name === testName);
}

async function openForm() {
    await $('header button[class*="_createButton_"]').click();
    await $(dialog + '.visible').waitForDisplayed();
    await browser.waitUntil(async () => (await $$(dialog + ' .CodeMirror')).length === 3);
}

async function action(text) {
    await $(`//div[contains(@class,"datavisool-dialog")]//footer//button[normalize-space()="${text}"]`).click();
}

async function editor(index, value = null) {
    return browser.execute((index, value) => {
        const instance = document.querySelectorAll('.datavisool-dialog .CodeMirror')[index].CodeMirror;
        if (value !== null) instance.setValue(value);
        return instance.getValue();
    }, index, value);
}

describe('API 管理前端直连', function () {
    this.timeout(60000);

    before(async () => {
        await browser.switchToWindow('main');
        await browser.execute(() => window.__TAURI__.window.getCurrentWindow().setFocus());
        await $('//button[contains(@class,"datavisool-menu-submenu-trigger")][contains(.,"数据源管理")]').click();
        await $('.datavisool-app-menu-node-api button').click();
        await browser.waitUntil(() => browser.execute(() => location.pathname === '/home/api'));

        server = createServer(async (req, res) => {
            res.setHeader('Access-Control-Allow-Origin', '*');
            res.setHeader('Access-Control-Allow-Headers', 'content-type, x-probe');
            res.setHeader('Access-Control-Allow-Methods', 'GET, POST, OPTIONS');
            if (req.method === 'OPTIONS') {
                res.writeHead(204).end();
                return;
            }
            let body = '';
            for await (const part of req) body += part;
            if (req.url === '/error') {
                res.writeHead(503, {'Content-Type': 'application/json'}).end('{"message":"temporarily unavailable"}');
                return;
            }
            res.writeHead(200, {'Content-Type': 'application/json'});
            res.end(JSON.stringify({method: req.method, path: req.url, probe: req.headers['x-probe'], body}));
        });
        await new Promise((resolve, reject) => {
            server.once('error', reject);
            server.listen(0, '127.0.0.1', resolve);
        });
        origin = `http://127.0.0.1:${server.address().port}`;
    });

    after(async () => {
        if (server?.listening) await new Promise(resolve => server.close(resolve));
        const records = await matchingRecords();
        if (records.length) {
            const result = await api('/api/commonApi/batchDel', records.map(row => row.id));
            assert.equal(result.code, 200);
        }
    });

    it('sends GET and POST to the echo service and shows the current response', async () => {
        await openForm();
        await $('input[placeholder="请输入 API 名称"]').setValue(testName);
        await $(address).setValue(origin + '/echo?existing=keep');
        await editor(0, '{"X-Probe":"desktop"}');
        await editor(1, '{"added":"new value"}');
        await action('测试');
        await browser.waitUntil(async () => (await editor(2)).includes('"GET"'));
        const get = JSON.parse(await editor(2));
        assert.equal(get.probe, 'desktop');
        assert.deepEqual([...new URL(get.path, origin).searchParams], [['existing', 'keep'], ['added', 'new value']]);

        await $(dialog + ' [role="combobox"]').click();
        await $('//*[@role="option" and normalize-space()="POST"]').click();
        await $(address).setValue(origin + '/echo');
        await action('测试');
        await browser.waitUntil(async () => (await editor(2)).includes('"POST"'));
        const post = JSON.parse(await editor(2));
        assert.equal(post.probe, 'desktop');
        assert.deepEqual(JSON.parse(post.body), {added: 'new value'});

        await $(address).setValue(origin + '/error');
        await action('测试');
        await browser.waitUntil(async () => (await editor(2)).includes('503'));
        assert.doesNotMatch(await editor(2), /"probe"/);
        await $(address).setValue(origin + '/echo');
        await action('测试');
        await browser.waitUntil(async () => (await editor(2)).includes('"POST"'));
        await action('创建');
        await $(dialog).waitForExist({reverse: true});
        assert.equal((await matchingRecords()).length, 1);
        await openForm();
        assert.equal(await editor(2), '');
        await action('取消');
    });
});
