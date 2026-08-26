import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { promisify } from 'node:util';
import test from 'node:test';

const execFileAsync = promisify(execFile);
const require = createRequire(import.meta.url);
const {
  monthlyRecurringRevenueUsd,
} = require('../dist/agent/domain.js');

test('MRR counts active recurring package prices once', () => {
  const today = new Date('2026-08-25T00:00:00.000Z');
  const base = {
    status: 'ACTIVE',
    priceBasis: 'RECURRING',
    price: { amount: 1200, currencyCode: 'USD' },
    effectiveFrom: '2026-01-01',
  };

  assert.equal(
    monthlyRecurringRevenueUsd({ ...base, billingCadence: 'MONTHLY' }, today),
    1200,
  );
  assert.equal(
    monthlyRecurringRevenueUsd({ ...base, billingCadence: 'QUARTERLY' }, today),
    400,
  );
  assert.equal(
    monthlyRecurringRevenueUsd({ ...base, billingCadence: 'ANNUAL' }, today),
    100,
  );
  assert.equal(
    monthlyRecurringRevenueUsd(
      { ...base, priceBasis: 'ONE_TIME', billingCadence: 'NONE' },
      today,
    ),
    0,
  );
  assert.equal(
    monthlyRecurringRevenueUsd(
      { ...base, billingCadence: 'MONTHLY', effectiveTo: '2026-06-30' },
      today,
    ),
    0,
  );
});

test('the agent command applies and reads the ordinary NED path idempotently', async (t) => {
  const records = new Map();
  let nextId = 1;

  const server = createServer(async (request, response) => {
    if (request.headers.authorization !== 'Bearer test-key') {
      response.writeHead(401, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ error: 'unauthorized' }));
      return;
    }

    const url = new URL(request.url, 'http://127.0.0.1');
    const pathParts = url.pathname.split('/').filter(Boolean);
    if (pathParts[0] !== 'rest' || !pathParts[1]) {
      response.writeHead(404).end();
      return;
    }

    const resource = pathParts[1];
    const collection = records.get(resource) ?? [];
    records.set(resource, collection);

    if (request.method === 'GET') {
      const filter = url.searchParams.get('filter');
      let matching = collection;
      if (filter) {
        const match = /^([A-Za-z][A-Za-z0-9]*)\[eq\]:(.*)$/.exec(filter);
        assert.ok(match, `unsupported test filter: ${filter}`);
        matching = collection.filter(
          (record) => String(record[match[1]]) === match[2],
        );
      }
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ data: { [resource]: matching } }));
      return;
    }

    let body = '';
    for await (const chunk of request) body += chunk;
    const input = body ? JSON.parse(body) : {};

    if (request.method === 'POST') {
      const created = { id: `${resource}-${nextId++}`, ...input };
      collection.push(created);
      response.writeHead(201, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ data: created }));
      return;
    }

    if (request.method === 'PATCH' && pathParts[2]) {
      const index = collection.findIndex((record) => record.id === pathParts[2]);
      assert.notEqual(index, -1, `record ${pathParts[2]} must exist`);
      collection[index] = { ...collection[index], ...input };
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ data: collection[index] }));
      return;
    }

    response.writeHead(405).end();
  });

  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(() => server.close());
  const address = server.address();
  assert.ok(address && typeof address !== 'string');

  const env = {
    ...process.env,
    FINITE_COMMERCIAL_TWENTY_URL: `http://127.0.0.1:${address.port}`,
    FINITE_COMMERCIAL_TWENTY_API_KEY: 'test-key',
  };
  const fixturePath = new URL('./fixtures/ned-update.json', import.meta.url);
  const run = (...args) =>
    execFileAsync(process.execPath, ['dist/agent/cli.js', ...args], { env });

  const first = JSON.parse(
    (await run('apply', '--file', fixturePath.pathname)).stdout,
  );
  assert.equal(first.organization.name, 'NED');
  assert.equal(first.metrics.currentMrrUsd, 0);
  assert.equal(first.metrics.lifetimeNetCashUsd, 100);
  assert.equal(first.metrics.isCurrentCustomer, false);
  assert.equal(first.changes.created, 10);
  assert.equal(first.changes.updated, 1);

  const countsAfterFirstApply = new Map(
    [...records].map(([resource, values]) => [resource, values.length]),
  );
  const second = JSON.parse(
    (await run('apply', '--file', fixturePath.pathname)).stdout,
  );
  assert.equal(second.changes.created, 0);
  assert.deepEqual(
    new Map([...records].map(([resource, values]) => [resource, values.length])),
    countsAfterFirstApply,
  );

  const shown = JSON.parse(
    (await run('show', '--organization', 'NED')).stdout,
  );
  assert.equal(shown.organization.name, 'NED');
  assert.equal(shown.metrics.lifetimeNetCashUsd, 100);
  assert.deepEqual(
    shown.openOpportunities.map((opportunity) => opportunity.name),
    ['NED follow-on work'],
  );
  assert.deepEqual(
    shown.purchases.map((purchase) => purchase.name),
    ['NED Agent Camp package'],
  );
  assert.deepEqual(
    shown.contacts.map((contact) => contact.email),
    ['ned-contact@example.invalid'],
  );
});
