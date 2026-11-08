import assert from 'node:assert/strict';
import {constants, createCipheriv, publicEncrypt, randomBytes} from 'node:crypto';

// Opt-in: pass temporary database credentials through DATAVISOOL_DB_TEST_CONFIG.
const configurations = JSON.parse(process.env.DATAVISOOL_DB_TEST_CONFIG || '[]');
const prefix = `数据库真实回归-${Date.now()}`;
const ids = [];

async function request(url, method = 'POST', body = {current: 1, size: 100}) {
    return browser.execute(async (url, method, body) => window.__TAURI__.core.invoke('datavisool_http_request', {
        req: {url, method, body},
    }), url, method, body);
}

async function encrypt(data) {
    const key = await request('/api/security/getEncryptKey', 'GET', null);
    assert.equal(key.code, 200);
    const aes = randomBytes(32);
    const iv = randomBytes(16);
    const cipher = createCipheriv('aes-256-cbc', aes, iv);
    return {
        encryptedData: Buffer.concat([cipher.update(JSON.stringify(data), 'utf8'), cipher.final()]).toString('base64'),
        encryptedAesKey: publicEncrypt({key: key.data.key, padding: constants.RSA_PKCS1_PADDING}, Buffer.from(aes.toString('base64'))).toString('base64'),
        iv: iv.toString('base64'),
    };
}

async function save(data) {
    const response = await request(data.id ? '/api/commonDatabase/update' : '/api/commonDatabase/add', 'POST', await encrypt(data));
    assert.equal(response.code, 200, response.msg);
    if (!data.id) {
        data.id = response.data;
        ids.push(response.data);
    } else assert.equal(response.data, true);
}

async function execute(id, statement) {
    return request('/api/db/executor/execute', 'POST', {id, statement: Buffer.from(statement).toString('base64')});
}

function success(response, context) {
    assert.equal(response.code, 200, `${context}: ${response.msg}`);
    return response.data;
}

describe('real database regressions (opt-in)', function () {
    this.timeout(120000);
    if (!configurations.length) {
        it('requires temporary database configuration', function () { this.skip(); });
        return;
    }
    before(async () => {
        await $('//button[contains(@class,"datavisool-menu-submenu-trigger")][contains(.,"数据源管理")]').click();
        await $('.datavisool-app-menu-node-database button').click();
        await browser.waitUntil(async () => await browser.execute(() => location.pathname === '/home/database'), {timeout: 10000});
        await browser.pause(800);
    });
    after(async () => {
        if (ids.length) {
            const result = await request('/api/commonDatabase/batchDel', 'POST', ids);
            assert.equal(result.code, 200);
        }
        const list = await request('/api/commonDatabase/pageList');
        assert.equal(list.data.records.filter(item => item.name?.startsWith(prefix)).length, 0);
    });

    for (const config of configurations) {
        describe(config.type, function () {
            const suffix = String(Date.now()).slice(-8);
            const table = `DV_T${suffix}`;
            const fn = `DV_F${suffix}`;
            const proc = `DV_P${suffix}`;
            const data = {
                name: `${prefix}-${config.type}`, type: config.type, ip: '127.0.0.1', port: String(config.port),
                databaseName: config.database, username: config.username, password: config.password,
                url: config.url || '', allowSqlScript: false,
            };
            before(async () => { await save(data); });

            it('connects and enforces the basic SQL permission switch', async () => {
                success(await request(`/api/commonDatabase/test/${data.id}`, 'GET', null), 'connection');
                const search = await $('input[placeholder="搜索数据源名称/类型/描述"]');
                await search.setValue(data.name);
                await browser.keys('Enter');
                await browser.waitUntil(async () => await browser.execute(name => document.querySelector('main')?.innerText.includes(name), data.name), {timeout: 10000});
                await $('//article[contains(@class,"_dataSourceCard_")]//button[normalize-space()="测试"]').click();
                await browser.waitUntil(async () => await browser.execute(() => document.body.innerText.includes('连接正常')), {timeout: 30000});
                const select = config.type === 'Oracle' ? 'SELECT 42 AS value FROM dual' : 'SELECT 42 AS value';
                assert.equal(success(await execute(data.id, select), 'select').length, 1);
                assert.equal((await execute(data.id, `CREATE TABLE ${table} (id INTEGER)`)).code, 400);
                data.allowSqlScript = true;
                await save(data);
                success(await execute(data.id, `CREATE TABLE ${table} (id INTEGER)`), 'create table');
                success(await execute(data.id, `INSERT INTO ${table} (id) VALUES (7)`), 'insert');
                const rows = success(await execute(data.id, `SELECT id FROM ${table}`), 'read');
                assert.equal(rows[0][config.type === 'Oracle' ? 'ID' : 'id'], 7);
                data.allowSqlScript = false;
                await save(data);
                assert.equal((await execute(data.id, `INSERT INTO ${table} (id) VALUES (8)`)).code, 400);
            });

            if (config.type === 'PostgresSQL') {
                it('decodes all integer widths and null without changing values', async () => {
                    const rows = success(await execute(data.id, 'SELECT CAST(-12 AS SMALLINT) AS s, CAST(2147483647 AS INTEGER) AS i, CAST(2147483648 AS BIGINT) AS b, CAST(NULL AS INTEGER) AS missing'), 'integer types');
                    assert.deepEqual(rows, [{s: -12, i: 2147483647, b: 2147483648, missing: null}]);
                });
                it('prevents a function called by SELECT from writing when scripts are disabled', async () => {
                    data.allowSqlScript = true;
                    await save(data);
                    success(await execute(data.id, `CREATE FUNCTION ${fn}() RETURNS BIGINT LANGUAGE plpgsql AS $$ BEGIN INSERT INTO ${table} (id) VALUES (9); RETURN 9; END $$`), 'create function');
                    data.allowSqlScript = false;
                    await save(data);
                    const blocked = await execute(data.id, `SELECT ${fn}()`);
                    assert.notEqual(blocked.code, 200, 'write-capable function must be blocked');
                    const before = success(await execute(data.id, `SELECT COUNT(*) AS count FROM ${table}`), 'count before');
                    assert.equal(before[0].count, 1);
                    const cte = await execute(data.id, `WITH changed AS (DELETE FROM ${table} RETURNING id) SELECT * FROM changed`);
                    assert.notEqual(cte.code, 200, 'write CTE must be blocked');
                    data.allowSqlScript = true;
                    await save(data);
                    const result = success(await execute(data.id, `SELECT ${fn}() AS result`), 'allowed function');
                    assert.equal(result[0].result, 9);
                    assert.equal(success(await execute(data.id, `SELECT COUNT(*) AS count FROM ${table}`), 'count after')[0].count, 2);
                });
            }
            if (config.type === 'MySQL') {
                it('creates and calls a stored procedure when scripts are enabled', async () => {
                    data.allowSqlScript = true;
                    await save(data);
                    success(await execute(data.id, `CREATE PROCEDURE ${proc}() BEGIN INSERT INTO ${table} (id) VALUES (9); SELECT id FROM ${table} ORDER BY id; END`), 'create procedure');
                    const rows = success(await execute(data.id, `CALL ${proc}()`), 'call procedure');
                    assert.deepEqual(rows.map(row => row.id), [7, 9]);
                    data.allowSqlScript = false;
                    await save(data);
                    assert.equal((await execute(data.id, `CALL ${proc}()`)).code, 400);
                    assert.equal(success(await execute(data.id, `SELECT COUNT(*) AS count FROM ${table}`), 'count')[0].count, 2);
                });
                it('prevents a write-capable function from writing in read-only mode', async () => {
                    data.allowSqlScript = true;
                    await save(data);
                    success(await execute(data.id, `CREATE FUNCTION ${fn}() RETURNS INTEGER DETERMINISTIC MODIFIES SQL DATA BEGIN INSERT INTO ${table} (id) VALUES (10); RETURN 10; END`), 'create function');
                    data.allowSqlScript = false;
                    await save(data);
                    const blocked = await execute(data.id, `SELECT ${fn}()`);
                    assert.notEqual(blocked.code, 200, 'write-capable function must be blocked');
                    assert.equal(success(await execute(data.id, `SELECT COUNT(*) AS count FROM ${table}`), 'count before')[0].count, 2);
                    data.allowSqlScript = true;
                    await save(data);
                    assert.equal(success(await execute(data.id, `SELECT ${fn}() AS result`), 'allowed function')[0].result, 10);
                    assert.equal(success(await execute(data.id, `SELECT COUNT(*) AS count FROM ${table}`), 'count after')[0].count, 3);
                });
            }
        });
    }
});
