import assert from 'node:assert/strict';
import { execFile } from 'node:child_process';
import { createServer } from 'node:http';
import { createRequire } from 'node:module';
import { promisify } from 'node:util';
import test from 'node:test';

const execFileAsync = promisify(execFile);
const require = createRequire(import.meta.url);
const {
  deriveMetrics,
  monthlyRecurringRevenueUsd,
  normalizedMonthlyRecurringRevenueUsd,
} = require('../dist/agent/domain.js');
const { parseCommercialUpdate } = require('../dist/agent/validation.js');

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
  assert.equal(
    normalizedMonthlyRecurringRevenueUsd({
      ...base,
      billingCadence: 'ANNUAL',
      effectiveFrom: '2027-01-01',
    }),
    100,
    'normalization must not freeze a future term at zero',
  );
});

test('unconverted non-USD cash remains unknown rather than becoming zero', () => {
  const metrics = deriveMetrics([], [], [
    {
      id: 'payment-1',
      name: 'BTC payment',
      status: 'RECEIVED',
      nativeAmount: 0.1,
      assetCode: 'BTC',
    },
  ]);

  assert.equal(metrics.lifetimeNetCashUsd, null);
});

test('financial facts require evidence or an explicit warning', () => {
  assert.throws(
    () =>
      parseCommercialUpdate({
        version: 1,
        organization: { name: 'Example' },
        account: { name: 'Example account' },
        arrangements: [
          {
            name: 'Arrangement',
            packages: [
              {
                name: 'Package',
                priceBasis: 'ONE_TIME',
                price: { amount: 10, currencyCode: 'USD' },
              },
            ],
          },
        ],
      }),
    /sourceReference or reconciliationWarning/,
  );
});

test('a price change can end one term and add the next without renaming the package', () => {
  assert.doesNotThrow(() =>
    parseCommercialUpdate({
      version: 1,
      organization: { name: 'Example' },
      account: { name: 'Example account' },
      arrangements: [
        {
          name: 'Hosted service',
          packages: [
            {
              name: 'Hosted agent',
              status: 'ACTIVE',
              priceBasis: 'RECURRING',
              priceTermKey: 'hosted-agent-2026-01',
              price: { amount: 100, currencyCode: 'USD' },
              billingCadence: 'MONTHLY',
              effectiveFrom: '2026-01-01',
              effectiveTo: '2026-08-31',
              sourceReference: 'test://old-term',
            },
            {
              name: 'Hosted agent',
              status: 'ACTIVE',
              priceBasis: 'RECURRING',
              priceTermKey: 'hosted-agent-2026-09',
              price: { amount: 150, currencyCode: 'USD' },
              billingCadence: 'MONTHLY',
              effectiveFrom: '2026-09-01',
              sourceReference: 'test://new-term',
            },
          ],
        },
      ],
    }),
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
  assert.equal(
    shown.arrangements[0].packages[0].charges[0].payments[0].sourceReference,
    'test://synthetic/ned/agent-camp/payment',
  );

  const packageRecords = records.get('purchasedPackages');
  packageRecords.push({
    ...packageRecords[0],
    id: 'purchasedPackages-ambiguous',
  });
  const beforeAmbiguousApply = JSON.stringify([...records]);
  await assert.rejects(
    run('apply', '--file', fixturePath.pathname),
    /ambiguous purchasedPackages match/,
  );
  assert.equal(
    JSON.stringify([...records]),
    beforeAmbiguousApply,
    'ambiguity must be detected before the first write',
  );
});
