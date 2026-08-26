import {
  deriveMetrics,
  moneyToTwenty,
  monthlyRecurringRevenueUsd,
  usdToTwenty,
} from './domain';
import { TwentyClient } from './twenty-client';
import type {
  CommercialMetrics,
  CommercialUpdate,
  ContactUpdate,
  TwentyRecord,
} from './types';

type ChangeKind = 'created' | 'updated';

interface Change {
  action: ChangeKind;
  resource: string;
  id: string;
  name: string;
}

interface OrganizationGraph {
  organization: TwentyRecord;
  contacts: TwentyRecord[];
  accounts: TwentyRecord[];
  opportunities: TwentyRecord[];
  arrangements: TwentyRecord[];
  packages: TwentyRecord[];
  offeringLines: TwentyRecord[];
  charges: TwentyRecord[];
  incomingPayments: TwentyRecord[];
}

export async function applyCommercialUpdate(
  client: TwentyClient,
  update: CommercialUpdate,
  at = new Date(),
): Promise<Record<string, unknown>> {
  const changes: Change[] = [];
  const organization = await upsertByName(
    client,
    'companies',
    update.organization.name,
    compact({
      name: update.organization.name,
      domainName:
        update.organization.domainName === undefined
          ? undefined
          : domainLinks(update.organization.domainName),
      commercialRoles: update.organization.commercialRoles,
      brainPage:
        update.organization.brainPage === undefined
          ? undefined
          : links(update.organization.brainPage),
      relationshipSummary: update.organization.relationshipSummary,
      sourceReference: update.organization.sourceReference,
      reconciliationWarning: update.organization.reconciliationWarning,
    }),
    {},
    changes,
  );

  let account: TwentyRecord | undefined;

  for (const contact of update.contacts ?? []) {
    await upsertContact(client, organization, contact, changes);
  }

  if (update.account) {
    account = await upsertByName(
      client,
      'commercialAccounts',
      update.account.name,
      compact({
        name: update.account.name,
        status: update.account.status,
        sourceReference: update.account.sourceReference,
        reconciliationWarning: update.account.reconciliationWarning,
        organizationId: organization.id,
      }),
      { organizationId: organization.id },
      changes,
    );
  }

  const opportunitiesByName = new Map<string, TwentyRecord>();
  for (const opportunity of update.opportunities ?? []) {
    const record = await upsertByName(
      client,
      'opportunities',
      opportunity.name,
      compact({
        name: opportunity.name,
        companyId: organization.id,
        commercialStage: opportunity.stage,
        amount:
          opportunity.amount === undefined
            ? undefined
            : moneyToTwenty(opportunity.amount),
        brainWants:
          opportunity.brainWants === undefined
            ? undefined
            : links(opportunity.brainWants),
        sourceReference: opportunity.sourceReference,
        reconciliationWarning: opportunity.reconciliationWarning,
      }),
      { companyId: organization.id },
      changes,
    );
    opportunitiesByName.set(opportunity.name, record);
  }

  if ((update.arrangements?.length ?? 0) > 0 && account === undefined) {
    throw new Error('account is required to apply arrangements');
  }

  for (const arrangement of update.arrangements ?? []) {
    let wonOpportunity: TwentyRecord | undefined;
    if (arrangement.wonOpportunity) {
      wonOpportunity =
        opportunitiesByName.get(arrangement.wonOpportunity) ??
        (await findOneByName(
          client,
          'opportunities',
          arrangement.wonOpportunity,
          { companyId: organization.id },
        ));
      if (!wonOpportunity) {
        throw new Error(
          `won opportunity ${JSON.stringify(arrangement.wonOpportunity)} was not found`,
        );
      }
    }

    const arrangementRecord = await upsertByName(
      client,
      'commercialArrangements',
      arrangement.name,
      compact({
        name: arrangement.name,
        status: arrangement.status,
        startsOn: arrangement.startsOn,
        endsOn: arrangement.endsOn,
        sourceReference: arrangement.sourceReference,
        reconciliationWarning: arrangement.reconciliationWarning,
        accountId: account?.id,
        wonOpportunityId: wonOpportunity?.id,
      }),
      { accountId: account?.id as string },
      changes,
    );

    for (const purchasedPackage of arrangement.packages ?? []) {
      const mrr = monthlyRecurringRevenueUsd(purchasedPackage, at);
      const packageRecord = await upsertByName(
        client,
        'purchasedPackages',
        purchasedPackage.name,
        compact({
          name: purchasedPackage.name,
          status: purchasedPackage.status,
          priceBasis: purchasedPackage.priceBasis,
          price:
            purchasedPackage.price === undefined
              ? undefined
              : moneyToTwenty(purchasedPackage.price),
          billingCadence: purchasedPackage.billingCadence,
          effectiveFrom: purchasedPackage.effectiveFrom,
          effectiveTo: purchasedPackage.effectiveTo,
          monthlyRecurringRevenueUsd: usdToTwenty(mrr),
          sourceReference: purchasedPackage.sourceReference,
          reconciliationWarning: purchasedPackage.reconciliationWarning,
          arrangementId: arrangementRecord.id,
        }),
        { arrangementId: arrangementRecord.id },
        changes,
      );

      for (const line of purchasedPackage.offeringLines ?? []) {
        await upsertByName(
          client,
          'offeringLines',
          line.name,
          compact({
            name: line.name,
            status: line.status,
            fulfillmentPath: line.fulfillmentPath,
            quantity: line.quantity,
            serviceStartsOn: line.serviceStartsOn,
            serviceEndsOn: line.serviceEndsOn,
            description: line.description,
            purchasedPackageId: packageRecord.id,
          }),
          { purchasedPackageId: packageRecord.id },
          changes,
        );
      }

      for (const charge of purchasedPackage.charges ?? []) {
        const chargeRecord = await upsertByName(
          client,
          'charges',
          charge.name,
          compact({
            name: charge.name,
            amount: moneyToTwenty(charge.amount),
            status: charge.status,
            chargedOn: charge.chargedOn,
            dueOn: charge.dueOn,
            sourceReference: charge.sourceReference,
            reconciliationWarning: charge.reconciliationWarning,
            accountId: account?.id,
            purchasedPackageId: packageRecord.id,
          }),
          {
            accountId: account?.id as string,
            purchasedPackageId: packageRecord.id,
          },
          changes,
        );

        for (const payment of charge.payments ?? []) {
          await upsertByName(
            client,
            'incomingPayments',
            payment.name,
            compact({
              name: payment.name,
              nativeAmount: payment.nativeAmount,
              assetCode: payment.assetCode.toUpperCase(),
              reportingValueUsd:
                payment.reportingValueUsd === undefined
                  ? undefined
                  : usdToTwenty(payment.reportingValueUsd),
              receivedAt: payment.receivedAt,
              status: payment.status,
              method: payment.method,
              transactionReference: payment.transactionReference,
              sourceReference: payment.sourceReference,
              reconciliationWarning: payment.reconciliationWarning,
              payerAccountId: account?.id,
              chargeId: chargeRecord.id,
            }),
            {
              payerAccountId: account?.id as string,
              chargeId: chargeRecord.id,
            },
            changes,
          );
        }
      }
    }
  }

  const graph = await loadOrganizationGraph(client, organization);
  const metrics = deriveMetrics(
    graph.packages,
    graph.offeringLines,
    graph.incomingPayments,
    at,
  );
  await client.update('companies', organization.id, {
    currentMrrUsd: usdToTwenty(metrics.currentMrrUsd),
    lifetimeNetCashUsd: usdToTwenty(metrics.lifetimeNetCashUsd),
    isCurrentCustomer: metrics.isCurrentCustomer,
  });
  changes.push({
    action: 'updated',
    resource: 'companies',
    id: organization.id,
    name: String(organization.name),
  });

  return {
    organization: { id: organization.id, name: organization.name },
    changes: changeSummary(changes),
    metrics,
    unresolvedFacts: reconciliationWarnings(graph),
  };
}

export async function showOrganization(
  client: TwentyClient,
  organizationName: string,
  at = new Date(),
): Promise<Record<string, unknown>> {
  const organization = await findOneByName(
    client,
    'companies',
    organizationName,
    {},
  );
  if (!organization) {
    throw new Error(`organization ${JSON.stringify(organizationName)} was not found`);
  }
  const graph = await loadOrganizationGraph(client, organization);
  const metrics = deriveMetrics(
    graph.packages,
    graph.offeringLines,
    graph.incomingPayments,
    at,
  );

  return {
    organization: select(organization, [
      'id',
      'name',
      'domainName',
      'commercialRoles',
      'relationshipSummary',
      'brainPage',
      'sourceReference',
    ]),
    accounts: graph.accounts.map((record) =>
      select(record, ['id', 'name', 'status']),
    ),
    contacts: graph.contacts
      .map((record) => contactForDisplay(record))
      .sort((left, right) =>
        `${String(left.lastName)} ${String(left.firstName)}`.localeCompare(
          `${String(right.lastName)} ${String(right.firstName)}`,
        ),
      ),
    currentServices: graph.offeringLines
      .filter((line) => line.status === 'ACTIVE')
      .map((line) => select(line, ['id', 'name', 'fulfillmentPath', 'quantity'])),
    purchases: graph.packages
      .map((record) =>
        select(record, [
          'id',
          'name',
          'status',
          'priceBasis',
          'price',
          'billingCadence',
          'effectiveFrom',
          'effectiveTo',
          'monthlyRecurringRevenueUsd',
        ]),
      )
      .sort(compareNames),
    payments: graph.incomingPayments
      .map((record) =>
        select(record, [
          'id',
          'name',
          'nativeAmount',
          'assetCode',
          'reportingValueUsd',
          'receivedAt',
          'status',
          'method',
          'transactionReference',
        ]),
      )
      .sort(compareNames),
    openOpportunities: graph.opportunities
      .filter(
        (opportunity) =>
          opportunity.commercialStage !== 'WON' &&
          opportunity.commercialStage !== 'LOST',
      )
      .map((record) =>
        select(record, [
          'id',
          'name',
          'commercialStage',
          'amount',
          'brainWants',
        ]),
      )
      .sort(compareNames),
    metrics,
    unresolvedFacts: reconciliationWarnings(graph),
  };
}

async function loadOrganizationGraph(
  client: TwentyClient,
  organization: TwentyRecord,
): Promise<OrganizationGraph> {
  const accounts = await client.list('commercialAccounts', {
    field: 'organizationId',
    value: organization.id,
  });
  const contacts = await client.list('people', {
    field: 'companyId',
    value: organization.id,
  });
  const opportunities = await client.list('opportunities', {
    field: 'companyId',
    value: organization.id,
  });
  const arrangements = await listChildren(
    client,
    'commercialArrangements',
    'accountId',
    accounts,
  );
  const packages = await listChildren(
    client,
    'purchasedPackages',
    'arrangementId',
    arrangements,
  );
  const offeringLines = await listChildren(
    client,
    'offeringLines',
    'purchasedPackageId',
    packages,
  );
  const charges = await listChildren(client, 'charges', 'accountId', accounts);
  const incomingPayments = await listChildren(
    client,
    'incomingPayments',
    'payerAccountId',
    accounts,
  );
  return {
    organization,
    contacts,
    accounts,
    opportunities,
    arrangements,
    packages,
    offeringLines,
    charges,
    incomingPayments,
  };
}

async function upsertContact(
  client: TwentyClient,
  organization: TwentyRecord,
  contact: ContactUpdate,
  changes: Change[],
): Promise<TwentyRecord> {
  const people = await client.list('people', {
    field: 'companyId',
    value: organization.id,
  });
  const normalizedEmail = contact.email?.toLowerCase();
  const matching = people.filter((person) => {
    if (normalizedEmail) return contactEmail(person)?.toLowerCase() === normalizedEmail;
    const name = asObject(person.name);
    return (
      name?.firstName === contact.firstName && name?.lastName === contact.lastName
    );
  });
  if (matching.length > 1) {
    throw new Error(
      `ambiguous people match for ${JSON.stringify(`${contact.firstName} ${contact.lastName}`)}; no record was changed`,
    );
  }
  const data = compact({
    name: { firstName: contact.firstName, lastName: contact.lastName },
    emails:
      contact.email === undefined
        ? undefined
        : { primaryEmail: contact.email, additionalEmails: [] },
    jobTitle: contact.jobTitle,
    linkedinLink:
      contact.linkedinUrl === undefined ? undefined : links(contact.linkedinUrl),
    companyId: organization.id,
  });
  if (matching[0]) {
    const updated = await client.update('people', matching[0].id, data);
    changes.push({
      action: 'updated',
      resource: 'people',
      id: updated.id,
      name: `${contact.firstName} ${contact.lastName}`,
    });
    return updated;
  }
  const created = await client.create('people', data);
  changes.push({
    action: 'created',
    resource: 'people',
    id: created.id,
    name: `${contact.firstName} ${contact.lastName}`,
  });
  return created;
}

async function listChildren(
  client: TwentyClient,
  resource: string,
  parentField: string,
  parents: TwentyRecord[],
): Promise<TwentyRecord[]> {
  const batches = await Promise.all(
    parents.map((parent) =>
      client.list(resource, { field: parentField, value: parent.id }),
    ),
  );
  return deduplicate(batches.flat());
}

async function upsertByName(
  client: TwentyClient,
  resource: string,
  name: string,
  data: Record<string, unknown>,
  scope: Record<string, string>,
  changes: Change[],
): Promise<TwentyRecord> {
  const existing = await findOneByName(client, resource, name, scope);
  if (existing) {
    const updated = await client.update(resource, existing.id, data);
    changes.push({ action: 'updated', resource, id: updated.id, name });
    return updated;
  }
  const created = await client.create(resource, data);
  changes.push({ action: 'created', resource, id: created.id, name });
  return created;
}

async function findOneByName(
  client: TwentyClient,
  resource: string,
  name: string,
  scope: Record<string, string>,
): Promise<TwentyRecord | undefined> {
  const firstScope = Object.entries(scope)[0];
  const candidates = await client.list(
    resource,
    firstScope
      ? { field: firstScope[0], value: firstScope[1] }
      : { field: 'name', value: name },
  );
  const matching = candidates.filter(
    (record) =>
      record.name === name &&
      Object.entries(scope).every(([field, value]) => record[field] === value),
  );
  if (matching.length > 1) {
    throw new Error(
      `ambiguous ${resource} match for ${JSON.stringify(name)}; no record was changed`,
    );
  }
  return matching[0];
}

function changeSummary(changes: Change[]): Record<string, unknown> {
  return {
    created: changes.filter((change) => change.action === 'created').length,
    updated: changes.filter((change) => change.action === 'updated').length,
    records: changes,
  };
}

function reconciliationWarnings(graph: OrganizationGraph): string[] {
  const resources: Array<[string, TwentyRecord[]]> = [
    ['organization', [graph.organization]],
    ['account', graph.accounts],
    ['opportunity', graph.opportunities],
    ['arrangement', graph.arrangements],
    ['package', graph.packages],
    ['charge', graph.charges],
    ['payment', graph.incomingPayments],
  ];
  return resources.flatMap(([kind, records]) =>
    records
      .filter((record) => record.reconciliationWarning === true)
      .map((record) => `${kind}: ${String(record.name ?? record.id)}`),
  );
}

function contactForDisplay(record: TwentyRecord): Record<string, unknown> {
  const name = asObject(record.name);
  return compact({
    id: record.id,
    firstName: name?.firstName,
    lastName: name?.lastName,
    email: contactEmail(record),
    jobTitle: record.jobTitle,
    linkedinLink: record.linkedinLink,
  });
}

function contactEmail(record: TwentyRecord): string | undefined {
  const emails = asObject(record.emails);
  return typeof emails?.primaryEmail === 'string'
    ? emails.primaryEmail
    : undefined;
}

function asObject(value: unknown): Record<string, unknown> | undefined {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function domainLinks(domain: string): Record<string, unknown> {
  const hostname = domain.replace(/^https?:\/\//, '').replace(/\/$/, '');
  return {
    primaryLinkLabel: hostname,
    primaryLinkUrl: `https://${hostname}`,
    secondaryLinks: [],
  };
}

function links(url: string): Record<string, unknown> {
  return {
    primaryLinkLabel: url,
    primaryLinkUrl: url,
    secondaryLinks: [],
  };
}

function compact(
  value: Record<string, unknown>,
): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(value).filter(([, fieldValue]) => fieldValue !== undefined),
  );
}

function deduplicate(records: TwentyRecord[]): TwentyRecord[] {
  return [...new Map(records.map((record) => [record.id, record])).values()];
}

function select(
  record: TwentyRecord,
  fields: string[],
): Record<string, unknown> {
  return compact(
    Object.fromEntries(fields.map((field) => [field, record[field]])),
  );
}

function compareNames(
  left: Record<string, unknown>,
  right: Record<string, unknown>,
): number {
  return String(left.name).localeCompare(String(right.name));
}
