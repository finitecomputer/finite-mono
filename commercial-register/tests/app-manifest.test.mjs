import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const manifestPath = new URL(
  '../.twenty/output/manifest.json',
  import.meta.url,
);
const manifest = JSON.parse(await readFile(manifestPath, 'utf8'));

const COMPANY_UNIVERSAL_IDENTIFIER =
  '20202020-b374-4779-a561-80086cb2e17f';
const OPPORTUNITY_UNIVERSAL_IDENTIFIER =
  '20202020-9549-49dd-b2b2-883999db8938';

test('the installed app exposes organizations and their commercial accounts', () => {
  const account = manifest.objects.find(
    (object) => object.nameSingular === 'commercialAccount',
  );
  assert.ok(account, 'Commercial Account must be an app-owned object');

  const companyFields = manifest.fields.filter(
    (field) =>
      field.objectUniversalIdentifier === COMPANY_UNIVERSAL_IDENTIFIER,
  );
  const companyFieldNames = companyFields.map((field) => field.name);

  assert.deepEqual(
    [
      'brainPage',
      'commercialAccounts',
      'commercialRoles',
      'currentMrrUsd',
      'isCurrentCustomer',
      'lifetimeNetCashUsd',
      'reconciliationWarning',
      'relationshipSummary',
      'sourceReference',
    ].sort(),
    companyFieldNames.sort(),
  );

  for (const fieldName of [
    'currentMrrUsd',
    'isCurrentCustomer',
    'lifetimeNetCashUsd',
  ]) {
    const field = companyFields.find((candidate) => candidate.name === fieldName);
    assert.equal(field.isUIEditable, false, `${fieldName} must be derived`);
  }

  assert.ok(
    manifest.fields.some(
      (field) =>
        field.objectUniversalIdentifier === account.universalIdentifier &&
        field.name === 'organization',
    ),
    'Commercial Account must belong to a Company',
  );
});

test('the installed app represents won purchases and open follow-on work', () => {
  const objectsByName = new Map(
    manifest.objects.map((object) => [object.nameSingular, object]),
  );

  for (const name of [
    'commercialArrangement',
    'offeringLine',
    'purchasedPackage',
  ]) {
    assert.ok(objectsByName.has(name), `${name} must be an app-owned object`);
  }

  const opportunityFields = manifest.fields.filter(
    (field) =>
      field.objectUniversalIdentifier === OPPORTUNITY_UNIVERSAL_IDENTIFIER,
  );
  assert.deepEqual(
    [
      'brainWants',
      'commercialArrangements',
      'commercialStage',
      'reconciliationWarning',
      'sourceReference',
    ].sort(),
    opportunityFields.map((field) => field.name).sort(),
  );

  const stage = opportunityFields.find(
    (field) => field.name === 'commercialStage',
  );
  assert.deepEqual(
    [
      'EXPLORING',
      'LOST',
      'PAUSED',
      'PROPOSAL_DRAFTED',
      'PROPOSAL_SENT',
      'WON',
    ],
    stage.options.map((option) => option.value).sort(),
  );

  const arrangement = objectsByName.get('commercialArrangement');
  const purchasedPackage = objectsByName.get('purchasedPackage');
  const offeringLine = objectsByName.get('offeringLine');

  assert.ok(
    manifest.fields.some(
      (field) =>
        field.objectUniversalIdentifier === arrangement.universalIdentifier &&
        field.name === 'account',
    ),
    'An Arrangement must belong to a Commercial Account',
  );
  assert.ok(
    manifest.fields.some(
      (field) =>
        field.objectUniversalIdentifier === purchasedPackage.universalIdentifier &&
        field.name === 'arrangement',
    ),
    'A Purchased Package must belong to an Arrangement',
  );
  assert.ok(
    manifest.fields.some(
      (field) =>
        field.objectUniversalIdentifier === offeringLine.universalIdentifier &&
        field.name === 'purchasedPackage',
    ),
    'An Offering Line must belong to a Purchased Package',
  );

  const packageMrr = purchasedPackage.fields.find(
    (field) => field.name === 'monthlyRecurringRevenueUsd',
  );
  assert.equal(packageMrr.isUIEditable, false);
});

test('the installed app keeps charges separate from cash received', () => {
  const objectsByName = new Map(
    manifest.objects.map((object) => [object.nameSingular, object]),
  );
  const charge = objectsByName.get('charge');
  const incomingPayment = objectsByName.get('incomingPayment');
  const purchasedPackage = objectsByName.get('purchasedPackage');

  assert.ok(charge, 'Charge must be an app-owned object');
  assert.ok(incomingPayment, 'Incoming Payment must be an app-owned object');

  assert.ok(
    charge.fields.some((field) => field.name === 'amount'),
    'A Charge must record the amount requested',
  );
  assert.deepEqual(
    ['OPEN', 'PAID', 'VOID'],
    charge.fields
      .find((field) => field.name === 'status')
      .options.map((option) => option.value)
      .sort(),
  );

  for (const fieldName of [
    'assetCode',
    'nativeAmount',
    'receivedAt',
    'reportingValueUsd',
    'sourceReference',
    'status',
  ]) {
    assert.ok(
      incomingPayment.fields.some((field) => field.name === fieldName),
      `Incoming Payment must expose ${fieldName}`,
    );
  }

  assert.ok(
    manifest.fields.some(
      (field) =>
        field.objectUniversalIdentifier === charge.universalIdentifier &&
        field.name === 'account',
    ),
    'A Charge must belong to the Account expected to pay',
  );
  assert.ok(
    manifest.fields.some(
      (field) =>
        field.objectUniversalIdentifier === charge.universalIdentifier &&
        field.name === 'purchasedPackage' &&
        field.relationTargetObjectMetadataUniversalIdentifier ===
          purchasedPackage.universalIdentifier,
    ),
    'A Charge may identify what was purchased',
  );
  assert.ok(
    manifest.fields.some(
      (field) =>
        field.objectUniversalIdentifier === incomingPayment.universalIdentifier &&
        field.name === 'payerAccount',
    ),
    'An Incoming Payment must identify who paid Finite',
  );
  assert.ok(
    manifest.fields.some(
      (field) =>
        field.objectUniversalIdentifier === incomingPayment.universalIdentifier &&
        field.name === 'charge',
    ),
    'The ordinary MVP path may apply a payment directly to one Charge',
  );
});

test('the installed app provides the three MVP operating views', () => {
  const viewsByName = new Map(manifest.views.map((view) => [view.name, view]));

  for (const name of [
    'Current customers',
    'Open opportunities',
    'Organization directory',
  ]) {
    assert.ok(viewsByName.has(name), `${name} view must be installed`);
    assert.ok(
      manifest.navigationMenuItems.some(
        (item) => item.viewUniversalIdentifier === viewsByName.get(name).universalIdentifier,
      ),
      `${name} must be reachable from the Twenty sidebar`,
    );
  }

  const currentCustomers = viewsByName.get('Current customers');
  assert.ok(
    currentCustomers.filters.some(
      (filter) => filter.value === true && filter.operand === 'IS',
    ),
    'Current customers must be driven by the derived customer marker',
  );

  const openOpportunities = viewsByName.get('Open opportunities');
  assert.equal(openOpportunities.type, 'KANBAN');
  assert.equal(
    openOpportunities.mainGroupByFieldMetadataUniversalIdentifier,
    '3b0feccb-187a-46fa-905a-b07721ba0a95',
  );
  assert.deepEqual(
    ['LOST', 'WON'],
    openOpportunities.filters.map((filter) => filter.value).sort(),
  );
});
