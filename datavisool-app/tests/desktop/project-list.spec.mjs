import assert from 'node:assert/strict';

const prefix = `项目列表回归-${Date.now()}`;
const projectIds = [];

async function request(url, method = 'POST', body = {current: 1, size: 100}) {
    return browser.execute(async (url, method, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method, body},
    }), url, method, body);
}

async function openProjectList() {
    await $('.datavisool-app-menu-node-server button').click();
    await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/server'), {timeout: 10000});
    await $('.project-list-container').waitForDisplayed({timeout: 10000});
    await browser.pause(700);
}

async function search(name) {
    const field = await $('input[placeholder="搜索项目并回车"]');
    await field.setValue(name);
    await browser.keys('Enter');
    await browser.pause(500);
}

async function openMenu(action) {
    const trigger = await $('.project-cover-menu');
    await trigger.click();
    // The embedded driver click does not emit pointerdown, which opens Dropdown.
    await browser.keys('Enter');
    const item = await $(`//button[@role="menuitem"][.//span[text()="${action}"]]`);
    await item.waitForDisplayed({timeout: 5000});
    await item.click();
}

describe('project list regressions', function () {
    before(async () => {
        for (let index = 0; index < 7; index++) {
            const response = await request('/api/project/create', 'POST', {
                name: `${prefix}-${index}`,
                des: '',
                dataJson: JSON.stringify({canvasManager: {width: 1920, height: 1080}}),
            });
            assert.equal(response.code, 200);
            projectIds.push(response.data);
        }
        await openProjectList();
    });

    after(async () => {
        for (const id of projectIds) {
            const response = await request(`/api/project/del/${id}`, 'GET', null);
            assert.equal(response.code, 200);
        }
    });

    it('shows names and an honest placeholder for missing descriptions', async () => {
        await search(prefix);
        await browser.waitUntil(async () => (await $$('.project-list-item')).length === 6, {timeout: 10000});
        const card = await browser.execute(() => {
            const item = document.querySelector('.project-list-item');
            const name = item.querySelector('.project-name-content');
            return {nameWidth: name.clientWidth, description: item.querySelector('.project-description').textContent};
        });
        assert.ok(card.nameWidth >= 100, `Project name is too narrow: ${card.nameWidth}px`);
        assert.equal(card.description, '未填写项目描述');
    });

    it('rejects blank names in the creation dialog and backend', async () => {
        await $('.project-list-create-btn').click();
        const input = await $('.new-project-dialog input[placeholder="请输入项目名称"]');
        await input.waitForDisplayed({timeout: 5000});
        await input.setValue('  ');
        await $('.new-project-dialog .btn-create').click();
        assert.equal(await $('.new-project-dialog [role="alert"]').getText(), '项目名称不能为空');
        assert.equal(await browser.execute(() => location.pathname), '/home/server');
        const create = await request('/api/project/create', 'POST', {name: '  '});
        const subproject = await request('/api/project/createSubProject', 'POST', {name: '  ', pid: projectIds[0]});
        const update = await request('/api/project/update', 'POST', {id: projectIds[0], name: '  '});
        assert.equal(create.code, 400);
        assert.equal(subproject.code, 400);
        assert.equal(update.code, 400);
        const partialUpdate = await request('/api/project/update', 'POST', {id: projectIds[0], des: '测试描述'});
        assert.equal(partialUpdate.code, 200);
        await $('.new-project-dialog .btn-cancel').click();
    });

    it('does not overwrite a project name with whitespace', async () => {
        const original = `${prefix}-0`;
        await search(original);
        await browser.waitUntil(async () => (await $$('.project-list-item')).length === 1, {timeout: 10000});
        await openMenu('重命名');
        const input = await $('.project-rename-input input');
        await input.setValue('  ');
        await browser.keys('Enter');
        await browser.waitUntil(async () => await $('.project-name-content').getText() === original, {timeout: 5000});
        const response = await request(`/api/project/getProjectData/${projectIds[0]}`, 'GET', null);
        assert.equal(response.data.name, original);

        await openMenu('重命名');
        const validName = `${prefix}-已重命名`;
        await $('.project-rename-input input').setValue(validName);
        await browser.keys('Enter');
        await browser.waitUntil(async () => await $('.project-name-content').getText() === validName, {timeout: 5000});
        const renamed = await request(`/api/project/getProjectData/${projectIds[0]}`, 'GET', null);
        assert.equal(renamed.data.name, validName);
    });

    it('returns to the last valid page after deleting its only item', async () => {
        await search(prefix);
        await $('[aria-label="第 2 页"]').click();
        await browser.waitUntil(async () => (await $$('.project-list-item')).length === 1, {timeout: 10000});
        await openMenu('删除');
        const confirm = await $('//div[contains(@class,"datavisool-dialog") and contains(@class,"visible")][.//*[text()="删除确认"]]//button[normalize-space()="确认"]');
        await confirm.click();
        await browser.waitUntil(async () => (await $$('.project-list-item')).length === 6, {timeout: 10000});
        assert.equal(await $('.datavisool-pagination-item-active').getText(), '1');
        assert.ok((await $('.datavisool-pagination-total').getText()).includes('共 6 条'));
    });
});
