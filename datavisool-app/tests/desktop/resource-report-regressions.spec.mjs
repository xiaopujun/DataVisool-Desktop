import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
const prefix = `资源回归-${Date.now()}`;
const types = [['/api/font/pageList', '/api/font/batchDel'], ['/api/video/page', '/api/video/batchDelete'], ['/api/model/page', '/api/model/batchDelete'], ['/api/dynamicScript/pageList', '/api/dynamicScript/batchDel']];
async function req(url, body = {
  current: 1,
  size: 200
}) {
  return browser.execute(async (url, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
    req: {
      url,
      method: 'POST',
      body
    }
  }), url, body);
}
async function list(url) {
  const r = await req(url);
  assert.equal(r.code, 200);
  return r.data.records;
}
async function focusTestWindow() {
  await browser.execute(async () => {
    if (document.visibilityState !== 'visible' || !document.hasFocus()) {
      await window.__TAURI__.window.getCurrentWindow().setFocus();
    }
  });
  await browser.waitUntil(async () => await browser.execute(() => document.visibilityState === 'visible' && document.hasFocus()), {timeout: 10000});
}
async function go(label, path) {
  await $('.datavisool-app-menu-node-console button').click();
  await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/console' && document.querySelector('[aria-busy]')?.getAttribute('aria-busy') === 'false'), {
    timeout: 30000
  });
  await $(`button[aria-label="查看 ${label}"]`).click();
  await browser.waitUntil(async () => await browser.execute(p => location.pathname === p, path), {
    timeout: 10000
  });
  await browser.pause(500);
  await focusTestWindow();
}
async function addFile(name, type, text, drop = false) {
  await browser.execute((name, type, text, drop) => {
    const d = document.querySelector('[role=dialog]'),
      dt = new DataTransfer();
    dt.items.add(new File([text], name, {
      type
    }));
    const input = d.querySelector('input[type=file]');
    if (drop) d.querySelector('.datavisool-upload-dragger').dispatchEvent(new DragEvent('drop', {
      bubbles: true,
      cancelable: true,
      dataTransfer: dt
    }));else {
      input.files = dt.files;
      input.dispatchEvent(new Event('change', {
        bubbles: true
      }));
    }
  }, name, type, text, drop);
}
describe('资源报告回归', function () {
  this.timeout(90000);
  before(async () => await $('.datavisool-app-menu-node-console button').waitForDisplayed({
    timeout: 30000
  }));
  after(async () => {
    for (const [page, del] of types) {
      const ids = (await list(page)).filter(x => x.name?.includes(prefix)).map(x => x.id);
      if (ids.length) assert.equal((await req(del, ids)).code, 200);
    }
  });
  it('字体搜索预览与空名称校验', async () => {
    const r = await req('/api/font/upload', {
      fonts: [{
        fileName: `${prefix}.ttf`,
        dataUrl: 'data:font/ttf;base64,eA=='
      }]
    });
    assert.equal(r.code, 200);
    assert.equal((await req('/api/font/update', {
      id: r.data[0],
      name: '  '
    })).code, 400);
    await go('字体', '/home/font');
    await $('input[placeholder="搜索字体名称 / 预览内容"]').setValue('AaBbCc123');
    await browser.keys('Enter');
    await browser.waitUntil(async () => await browser.execute(p => [...document.querySelectorAll('article h2')].some(x => x.textContent.includes(p)), prefix), {
      timeout: 10000
    });
    const d = await browser.execute(() => {
      const p = document.querySelector('h1').parentElement.querySelector('p');
      return {
        h: p.clientHeight,
        s: p.scrollHeight
      };
    });
    assert.ok(d.h >= d.s);
    await $('//article//button[normalize-space()="编辑"]').click();
    await $('input[placeholder="请输入字体名称"]').setValue('   ');
    await $('//section[contains(@class,"datavisool-global-modal-card")]//button[normalize-space()="确定"]').click();
    await browser.waitUntil(async () => await browser.execute(() => document.body.innerText.includes('字体名称不能为空')), {
      timeout: 5000
    });
    await $('//section[contains(@class,"datavisool-global-modal-card")]//button[normalize-space()="取消"]').click();
  });
  it('视频播放入口与非视频文件校验', async () => {
    assert.equal((await req('/api/video/upload', {
      files: [{
        fileName: `${prefix}.txt`,
        mime: 'text/plain',
        dataUrl: 'data:text/plain;base64,eA=='
      }]
    })).code, 400);
    assert.equal((await req('/api/video/upload', {
      files: [{
        fileName: `${prefix}.mp4`,
        mime: 'video/mp4',
        dataUrl: 'data:video/mp4;base64,eA=='
      }]
    })).code, 200);
    await go('视频', '/home/videoResource');
    await browser.waitUntil(async () => (await $$('article[class*="videoCard_"]')).length > 0, {
      timeout: 10000
    });
    assert.ok((await $('article[class*="videoCard_"]').getText()).includes('1 B'));
    await $('button[aria-label="播放视频"]').click();
    await $('[role=dialog] video').waitForDisplayed({
      timeout: 5000
    });
    await $('[role=dialog] button[aria-label="关闭"]').click();
    await browser.pause(250);
    await $('//header//button[normalize-space()="上传"]').click();
    await $('[role=dialog]').waitForDisplayed({
      timeout: 5000
    });
    await addFile(`${prefix}.txt`, 'text/plain', 'not video', true);
    await browser.pause(250);
    assert.equal(await $('button[class*="modalUploadButton"]').isEnabled(), false);
    await $('//div[@role="dialog"]//button[normalize-space()="取消"]').click();
  });
  it('模型替换保持 ID，失败不破坏原记录', async () => {
    const r = await req('/api/model/upload', {
      files: [{
        fileName: `${prefix}.gltf`,
        mime: 'model/gltf+json',
        dataUrl: 'data:text/plain;base64,b2xk'
      }, {
        fileName: `${prefix}-shared.gltf`,
        mime: 'model/gltf+json',
        dataUrl: 'data:text/plain;base64,b2xk'
      }]
    });
    assert.equal(r.code, 200);
    const id = r.data[0].id,
      sharedId = r.data[1].id,
      old = (await list('/api/model/page')).find(x => x.id === id);
    const bad = await req('/api/model/upload', {
      id,
      files: [{
        fileName: `${prefix}.zip`,
        mime: 'application/zip',
        dataUrl: 'data:application/zip;base64,eA=='
      }]
    });
    assert.equal(bad.code, 400);
    assert.equal((await list('/api/model/page')).find(x => x.id === id).url, old.url);
    await go('3D 模型', '/home/modelResource');
    await $('input[placeholder="搜索模型名称"]').setValue(`${prefix}.gltf`);
    await browser.keys('Enter');
    await browser.waitUntil(async () => (await $$('article[class*="modelCard_"]')).length === 1, {
      timeout: 10000
    });
    assert.ok((await $('article[class*="modelCard_"]').getText()).includes('3 B'));
    await $('//article//button[normalize-space()="替换"]').click();
    await $('[role=dialog]').waitForDisplayed({
      timeout: 5000
    });
    await addFile(`${prefix}-new.obj`, 'text/plain', 'replacement');
    await browser.waitUntil(async () => await $('button[class*="modalUploadButton"]').isEnabled(), {
      timeout: 5000
    });
    await $('button[class*="modalUploadButton"]').click();
    await browser.waitUntil(async () => (await list('/api/model/page')).some(x => x.id === id && x.url !== old.url), {
      timeout: 10000
    });
    const items = (await list('/api/model/page')).filter(x => x.name?.includes(prefix));
    assert.equal(items.length, 2);
    assert.equal(items.find(x => x.id === sharedId).url, old.url);
    await browser.waitUntil(async () => (await $('article[class*="modelCard_"]').getText()).includes('OBJ'), {timeout: 10000});
    const pathname = decodeURIComponent(new URL(old.url).pathname.slice(1));
    assert.equal(readFileSync(pathname, 'utf8'), 'old');
  });
  it('脚本地址、空名称及大写扩展名', async () => {
    assert.equal((await req('/api/dynamicScript/saveOrUpdate', {
      name: prefix,
      type: 'REMOTE',
      url: 'http://['
    })).code, 400);
    const r = await req('/api/dynamicScript/saveOrUpdate', {
      name: prefix,
      type: 'REMOTE',
      url: 'https://example.invalid/test.js'
    });
    assert.equal(r.code, 200);
    assert.equal((await req('/api/dynamicScript/update', {
      id: r.data,
      name: '  '
    })).code, 400);
    assert.equal((await req('/api/dynamicScript/update', {id: r.data, url: 'http://['})).code, 400);
    await go('动态脚本', '/home/dynamicScript');
    await $('//header//button[normalize-space()="创建"]').click();
    await $('[role=dialog]').waitForDisplayed({
      timeout: 5000
    });
    await $('input[placeholder="请输入脚本链接或 CDN 地址"]').setValue('http://[');
    await $('//div[@role="dialog"]//button[normalize-space()="添加"]').click();
    await browser.waitUntil(async () => await browser.execute(() => document.body.innerText.includes('请输入有效的脚本地址')), {
      timeout: 5000
    });
    await $('[role=dialog] [role=combobox]').click();
    await $('//*[@role="option" and normalize-space()="本地脚本"]').click();
    await addFile(`${prefix}.JS`, 'text/javascript', 'window.example=1');
    await $('[aria-label="移除已选择文件"]').waitForDisplayed({
      timeout: 5000
    });
    await $('//div[@role="dialog"]//button[normalize-space()="添加"]').click();
    await browser.waitUntil(async () => (await list('/api/dynamicScript/pageList')).some(x => x.name === `${prefix}.JS`), {
      timeout: 10000
    });
  });
});
