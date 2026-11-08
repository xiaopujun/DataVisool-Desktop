import assert from 'node:assert/strict';
import {constants, createCipheriv, publicEncrypt, randomBytes} from 'node:crypto';

const prefix = `发布管理回归-${Date.now()}`;
const firstName = `${prefix}-甲`;
const secondName = `${prefix}-乙`;
const renamed = `${prefix}-已修改`;
const password = 'release-test-4281';
const projectIds = [];
const releaseIds = [];
let firstId;
let secondId;

async function request(url, method = 'POST', body = {current: 1, size: 100}) {
    return browser.execute(async (url, method, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method, body},
    }), url, method, body);
}

async function list(body = {current: 1, size: 100}) {
    const response = await request('/api/release/pageList', 'POST', body);
    assert.equal(response.code, 200, JSON.stringify(response));
    return response.data;
}

async function record(id) {
    return (await list()).records.find(item => item.id === id);
}

async function waitRecord(id, predicate) {
    let found;
    await browser.waitUntil(async () => {
        found = await record(id);
        return predicate(found);
    }, {timeout: 12000, interval: 200});
    return found;
}

async function makeRelease(name, description) {
    const project = await request('/api/project/create', 'POST', {name, des: description, dataJson: '{}'});
    assert.equal(project.code, 200);
    projectIds.push(project.data);
    const release = await request('/api/release/release', 'POST', {
        releaseType: 'NEW', projectId: project.data, encrypt: false, dependencies: [],
    });
    assert.equal(release.code, 200);
    releaseIds.push(release.data);
    return release.data;
}

async function searchFor(value, expected) {
    await browser.execute(async () => {
        if (document.visibilityState !== 'visible' || !document.hasFocus())
            await window.__TAURI__.window.getCurrentWindow().setFocus();
    });
    await browser.waitUntil(async () => await browser.execute(() => document.visibilityState === 'visible' && document.hasFocus()),
        {timeout: 10000});
    await $('input[placeholder="搜索已发布项目"]').setValue(value);
    await browser.keys('Enter');
    await browser.waitUntil(async () => await browser.execute((term, count) => {
        const names = [...document.querySelectorAll('[class*="tableBody_"] [class*="projectName_"]')];
        return names.length === count && names.every(item => item.textContent.includes(term));
    }, value, expected), {timeout: 10000});
}

async function row(name) {
    return $(`//div[contains(@class,"tableBody_")]/div[.//div[@title="${name}"]]`);
}

async function action(name, label) {
    await (await row(name)).$(`.//button[normalize-space()="${label}"]`).click();
}

async function edit(name) {
    await action(name, '编辑');
    await $('[role="dialog"]').waitForDisplayed({timeout: 5000});
}

async function update() {
    await $('//div[@role="dialog"]//button[normalize-space()="更新"]').click();
}

async function confirm(label) {
    await $('.datavisool-global-modal-card').waitForDisplayed({timeout: 5000});
    await $(`//section[contains(@class,"datavisool-global-modal-card")]//button[normalize-space()="${label}"]`).click();
}

async function encryptedPassword(value) {
    const response = await request('/api/security/getEncryptKey', 'GET', null);
    const aes = randomBytes(32);
    const iv = randomBytes(16);
    const cipher = createCipheriv('aes-256-cbc', aes, iv);
    return {
        encryptedData: Buffer.concat([cipher.update(value, 'utf8'), cipher.final()]).toString('base64'),
        encryptedAesKey: publicEncrypt({key: response.data.key, padding: constants.RSA_PKCS1_PADDING},
            Buffer.from(aes.toString('base64'))).toString('base64'),
        iv: iv.toString('base64'),
    };
}

async function openPreview(name) {
    await action(name, '查看发布');
    await browser.waitUntil(async () => (await browser.getWindowHandles()).some(handle => handle !== 'main'),
        {timeout: 10000});
    await browser.switchToWindow((await browser.getWindowHandles()).find(handle => handle !== 'main'));
}

async function closePreview() {
    await browser.closeWindow();
    await browser.switchToWindow('main');
    await browser.execute(async () => window.__TAURI__.window.getCurrentWindow().setFocus());
}

describe('发布管理回归', function () {
    this.timeout(120000);

    before(async () => {
        await $('.datavisool-app-menu-node-releaseManagement button').waitForDisplayed({timeout: 30000});
        await $('.datavisool-app-menu-node-releaseManagement button').click();
        await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/releaseManagement'),
            {timeout: 10000});
        await browser.execute(async () => window.__TAURI__.window.getCurrentWindow().setFocus());
        await browser.pause(500);
        firstId = await makeRelease(firstName, '测试项目描述');
        await browser.pause(1100);
        secondId = await makeRelease(secondName, '第二个项目描述');
    });

    after(async () => {
        const handles = await browser.getWindowHandles();
        for (const handle of handles.filter(item => item !== 'main')) {
            await browser.switchToWindow(handle);
            await browser.closeWindow();
        }
        await browser.switchToWindow('main');
        if (releaseIds.length) await request('/api/release/batchDel', 'POST', releaseIds);
        for (const id of projectIds) await request(`/api/project/del/${id}`, 'GET', null);
    });

    afterEach(async () => {
        const handles = await browser.getWindowHandles();
        for (const handle of handles.filter(item => item !== 'main')) {
            await browser.switchToWindow(handle);
            await browser.closeWindow();
        }
        await browser.switchToWindow('main');
        await browser.execute(async () => window.__TAURI__.window.getCurrentWindow().setFocus());
    });

    it('显示描述、本地发布时间与完整操作列，并支持按发布时间排序', async () => {
        await searchFor(prefix, 2);
        const visual = await browser.execute(() => {
            const viewport = document.querySelector('[class*="tableScroll_"]').getBoundingClientRect();
            const actions = document.querySelector('[class*="tableBody_"] [class*="actionGroup_"]').getBoundingClientRect();
            return {viewport: {left: viewport.left, right: viewport.right}, actions: {left: actions.left, right: actions.right}};
        });
        assert.ok(visual.actions.left >= visual.viewport.left && visual.actions.right <= visual.viewport.right, JSON.stringify(visual));
        assert.ok((await (await row(firstName)).getText()).includes('测试项目描述'));
        const first = await record(firstId);
        const d = new Date(`${first.createTime.replace(' ', 'T')}Z`);
        const pad = value => String(value).padStart(2, '0');
        const expected = `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())} ${pad(d.getHours())}:${pad(d.getMinutes())}:${pad(d.getSeconds())}`;
        assert.ok((await (await row(firstName)).getText()).includes(expected));
        const getNames = () => browser.execute(() => [...document.querySelectorAll('[class*="projectName_"]')].map(item => item.textContent));
        assert.deepEqual(await getNames(), [secondName, firstName]);
        await $('button[aria-label="发布时间：降序"]').click();
        await browser.waitUntil(async () => (await getNames())[0] === firstName, {timeout: 10000});
        await $('button[aria-label="发布时间：升序"]').click();
        await browser.waitUntil(async () => (await getNames())[0] === secondName, {timeout: 10000});
        await browser.saveScreenshot('/private/tmp/release-management-regression.png');
    });

    it('取消编辑不更改列表，并正确保存名称、有效期和密码', async () => {
        await searchFor(firstName, 1);
        const original = await record(firstId);
        await edit(firstName);
        await $('[role="dialog"] input[type="text"]').setValue(`${prefix}-取消`);
        await $('[role="dialog"] input[type="number"]').setValue('23');
        await $('[role="dialog"] button[role="switch"]').click();
        await $('//div[@role="dialog"]//button[normalize-space()="取消"]').click();
        const unchanged = await (await row(firstName)).getText();
        assert.ok(unchanged.includes(firstName) && unchanged.includes('永久') && unchanged.includes('未加密'));
        assert.equal((await record(firstId)).name, firstName);
        await edit(firstName);
        await $('[role="dialog"] input[type="text"]').setValue('   ');
        await update();
        await browser.waitUntil(async () => await browser.execute(() => document.body.innerText.includes('名称不能为空')),
            {timeout: 5000});
        assert.equal(await $('[role="dialog"]').isDisplayed(), true);
        assert.equal((await record(firstId)).name, firstName);
        await $('[role="dialog"] input[type="text"]').setValue(`  ${renamed}  `);
        await $('[role="dialog"] input[type="number"]').setValue('7');
        await $('[role="dialog"] button[role="switch"]').click();
        await update();
        await browser.waitUntil(async () => await browser.execute(() => document.body.innerText.includes('请输入密码')),
            {timeout: 5000});
        assert.equal(await $('[role="dialog"]').isDisplayed(), true);
        await $('[role="dialog"] input[type="password"]').setValue(password);
        await update();
        const saved = await waitRecord(firstId, item => item?.name === renamed && item.encrypt && item.expirationDate === 7);
        await browser.waitUntil(async () => !(await $('[role="dialog"]').isDisplayed()), {timeout: 10000});
        assert.equal(saved.createTime, original.createTime);
        assert.equal((await request('/api/release/getReleaseData', 'POST', {id: firstId})).code, 401);
        assert.equal((await request('/api/release/getReleaseData', 'POST', {id: firstId, password: await encryptedPassword('wrong')})).code, 401);
        assert.equal((await request('/api/release/getReleaseData', 'POST', {id: firstId, password: await encryptedPassword(password)})).code, 200);
    });

    it('保留已有密码、清空有效期和处理零天期限', async () => {
        await searchFor(renamed, 1);
        await edit(renamed);
        await $('[role="dialog"] input[type="number"]').clearValue();
        await update();
        await waitRecord(firstId, item => item?.expirationDate === null && item.encrypt);
        await browser.waitUntil(async () => !(await $('[role="dialog"]').isDisplayed()), {timeout: 10000});
        assert.equal((await request('/api/release/getReleaseData', 'POST', {id: firstId, password: await encryptedPassword(password)})).code, 200);
        await searchFor(renamed, 1);
        await edit(renamed);
        await $('[role="dialog"] input[type="number"]').setValue('0');
        await update();
        await waitRecord(firstId, item => item?.expirationDate === 0);
        await browser.waitUntil(async () => !(await $('[role="dialog"]').isDisplayed()), {timeout: 10000});
        await searchFor(renamed, 1);
        assert.ok((await (await row(renamed)).getText()).includes('已过期'));
        assert.equal((await request(`/api/release/getReleaseInfo/${firstId}`, 'GET', null)).code, 410);
        await edit(renamed);
        await $('[role="dialog"] input[type="number"]').clearValue();
        await $('[role="dialog"] button[role="switch"]').click();
        await update();
        await waitRecord(firstId, item => item?.expirationDate === null && !item.encrypt);
        await browser.waitUntil(async () => !(await $('[role="dialog"]').isDisplayed()), {timeout: 10000});
        assert.equal((await request('/api/release/getReleaseData', 'POST', {id: firstId})).code, 200);
    });

    it('搜索清除隐藏选择，批量删除仅影响当前选择', async () => {
        await searchFor(renamed, 1);
        await (await row(renamed)).$('button[aria-label="选择项目"]').click();
        await searchFor(secondName, 1);
        await (await row(secondName)).$('button[aria-label="选择项目"]').click();
        await $('//button[normalize-space()="删除"]').click();
        assert.ok((await $('.datavisool-global-modal-card').getText()).includes('(1)'));
        await confirm('取消');
        assert.ok(await record(secondId));
        await $('//button[normalize-space()="删除"]').click();
        await confirm('确定');
        await waitRecord(secondId, item => !item);
        assert.ok(await record(firstId));
        await searchFor(renamed, 1);
        await action(renamed, '删除');
        await confirm('取消');
        assert.ok(await record(firstId));
        await action(renamed, '删除');
        await confirm('确定');
        await waitRecord(firstId, item => !item);
    });

    it('跨页排序、全选、翻页与新搜索重置', async () => {
        for (let index = 0; index < 12; index++) {
            const response = await request('/api/release/release', 'POST', {
                releaseType: 'NEW', projectId: projectIds[0], encrypt: false,
                dependencies: [{id: projectIds[0], name: `${prefix}-分页${index}`, projectJson: '{}'}],
            });
            assert.equal(response.code, 200);
            releaseIds.push(response.data);
        }
        await searchFor(`${prefix}-分页`, 10);
        const api = await list({current: 1, size: 10, searchValue: `${prefix}-分页`, sortDirection: 'desc'});
        const names = () => browser.execute(() => [...document.querySelectorAll('[class*="projectName_"]')].map(item => item.textContent));
        assert.deepEqual(await names(), api.records.map(item => item.name));
        await $('button[aria-label="全选"]').click();
        assert.equal((await $$('[class*="tableBody_"] button[class*="checkboxChecked"]')).length, 10);
        await $('[aria-label="第 2 页"]').click();
        await browser.waitUntil(async () => (await $$('[class*="tableBody_"] [class*="tableRow_"]')).length === 2, {timeout: 10000});
        assert.equal((await $$('[class*="tableBody_"] button[class*="checkboxChecked"]')).length, 0);
        await searchFor('分页', 10);
        assert.equal(await $('.datavisool-pagination-item-active').getText(), '1');
        await $('button[aria-label="发布时间：降序"]').click();
        const ascending = await list({current: 1, size: 10, searchValue: '分页', sortDirection: 'asc'});
        await browser.waitUntil(async () => JSON.stringify(await names()) === JSON.stringify(ascending.records.map(item => item.name)), {timeout: 10000});
        await $('[aria-label="第 2 页"]').click();
        const pageTwo = await list({current: 2, size: 10, searchValue: '分页', sortDirection: 'asc'});
        await browser.waitUntil(async () => JSON.stringify(await names()) === JSON.stringify(pageTwo.records.map(item => item.name)), {timeout: 10000});
    });
    it('状态开关和预览窗口反映实际访问状态', async () => {
        const previewName = `${prefix}-预览`;
        const previewId = await makeRelease(previewName, '预览测试');
        await searchFor(previewName, 1);
        const before = await record(previewId);
        await (await row(previewName)).$('button[role="switch"]').click();
        await waitRecord(previewId, item => item && !item.state);
        assert.equal((await request(`/api/release/getReleaseInfo/${previewId}`, 'GET', null)).code, 403);
        await openPreview(previewName);
        await browser.waitUntil(async () => await browser.execute(() => document.body.innerText.includes('该发布链接已停用')),
            {timeout: 12000});
        await closePreview();
        await (await row(previewName)).$('button[role="switch"]').click();
        await waitRecord(previewId, item => item?.state);
        assert.equal((await record(previewId)).createTime, before.createTime);
        await openPreview(previewName);
        await browser.waitUntil(async () => await browser.execute(() => !!document.querySelector('.datavisool-app-project-release')),
            {timeout: 20000});
        await closePreview();
    });


});
