import {
  deriveMetrics,
  moneyAmount,
  moneyToTwenty,
  normalizedMonthlyRecurringRevenueUsd,
  usdToTwenty,
} from './domain';
import { TwentyClient } from './twenty-client';
import type {
  CommercialArrangementUpdate,
  CommercialUpdate,
  ContactUpdate,
  PurchasedPackageUpdate,
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

interface PreflightPlan {
  matches: Map<object, TwentyRecord | undefined>;
  wonOpportunities: Map<CommercialArrangementUpdate, TwentyRecord>;
}

export async function applyCommercialUpdate(
  client: TwentyClient,
  update: CommercialUpdate,
  at = new Date(),
): Promise<Record<string, unknown>> {
  const plan = await preflightCommercialUpdate(client, update);
  const changes: Change[] = [];
  const organization = await persistPlannedRecord(
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
      relationshipSummaryRefreshedAt:
        update.organization.relationshipSummaryRefreshedAt,
      sourceReference: update.organization.sourceReference,
      reconciliationWarning: update.organization.reconciliationWarning,
    }),
    plan.matches.get(update.organization),
    changes,
  );

  let account: TwentyRecord | undefined;

  for (const contact of update.contacts ?? []) {
    await persistContact(
      client,
      organization,
      contact,
      plan.matches.get(contact),
      changes,
    );
  }

  if (update.account) {
    account = await persistPlannedRecord(
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
      plan.matches.get(update.account),
      changes,
    );
  }

  const opportunitiesByName = new Map<string, TwentyRecord>();
  for (const opportunity of update.opportunities ?? []) {
    const record = await persistPlannedRecord(
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
      plan.matches.get(opportunity),
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
        plan.wonOpportunities.get(arrangement);
      if (!wonOpportunity) {
        throw new Error(
          `won opportunity ${JSON.stringify(arrangement.wonOpportunity)} was not found`,
        );
      }
    }

    const arrangementRecord = await persistPlannedRecord(
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
      plan.matches.get(arrangement),
      changes,
    );

    for (const purchasedPackage of arrangement.packages ?? []) {
      const normalizedMrr = normalizedMonthlyRecurringRevenueUsd(purchasedPackage);
      const packageRecord = await persistPlannedRecord(
        client,
        'purchasedPackages',
        purchasedPackage.name,
        compact({
          name: purchasedPackage.name,
          status: purchasedPackage.status,
          priceBasis: purchasedPackage.priceBasis,
          priceTermKey: purchasedPackage.priceTermKey,
          price:
            purchasedPackage.price === undefined
              ? undefined
              : moneyToTwenty(purchasedPackage.price),
          billingCadence: purchasedPackage.billingCadence,
          effectiveFrom: purchasedPackage.effectiveFrom,
          effectiveTo: purchasedPackage.effectiveTo,
          monthlyRecurringRevenueUsd: usdToTwenty(normalizedMrr),
          sourceReference: purchasedPackage.sourceReference,
          reconciliationWarning: purchasedPackage.reconciliationWarning,
          arrangementId: arrangementRecord.id,
      }),
        plan.matches.get(purchasedPackage),
        changes,
      );

      for (const line of purchasedPackage.offeringLines ?? []) {
        await persistPlannedRecord(
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
          plan.matches.get(line),
          changes,
        );
      }

      for (const charge of purchasedPackage.charges ?? []) {
        const chargeRecord = await persistPlannedRecord(
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
          plan.matches.get(charge),
          changes,
        );

        for (const payment of charge.payments ?? []) {
          await persistPlannedRecord(
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
              network: payment.network,
              transactionReference: payment.transactionReference,
              sourceReference: payment.sourceReference,
              reconciliationWarning: payment.reconciliationWarning,
              payerAccountId: account?.id,
              chargeId: chargeRecord.id,
            }),
            plan.matches.get(payment),
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
    lifetimeNetCashUsd:
      metrics.lifetimeNetCashUsd === null
        ? null
        : usdToTwenty(metrics.lifetimeNetCashUsd),
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
      'relationshipSummaryRefreshedAt',
      'brainPage',
      'sourceReference',
    ]),
    accounts: graph.accounts.map((record) =>
      select(record, [
        'id',
        'name',
        'status',
        'sourceReference',
        'reconciliationWarning',
      ]),
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
      .map((line) => {
        const purchasedPackage = graph.packages.find(
          (candidate) => candidate.id === line.purchasedPackageId,
        );
        return compact({
          ...select(line, [
            'id',
            'name',
            'fulfillmentPath',
            'quantity',
            'purchasedPackageId',
          ]),
          sourceReference: purchasedPackage?.sourceReference,
          reconciliationWarning: purchasedPackage?.reconciliationWarning,
        });
      }),
    arrangements: arrangementsForDisplay(graph),
    purchases: graph.packages
      .map((record) =>
        select(record, [
          'id',
          'name',
          'status',
          'priceBasis',
          'priceTermKey',
          'price',
          'billingCadence',
          'effectiveFrom',
          'effectiveTo',
          'monthlyRecurringRevenueUsd',
          'arrangementId',
          'sourceReference',
          'reconciliationWarning',
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
          'network',
          'reportingValueUsd',
          'receivedAt',
          'status',
          'method',
          'transactionReference',
          'payerAccountId',
          'chargeId',
          'sourceReference',
          'reconciliationWarning',
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
          'sourceReference',
          'reconciliationWarning',
        ]),
      )
      .sort(compareNames),
    metrics,
    unresolvedFacts: reconciliationWarnings(graph),
  };
}

async function preflightCommercialUpdate(
  client: TwentyClient,
  update: CommercialUpdate,
): Promise<PreflightPlan> {
  const plan: PreflightPlan = {
    matches: new Map(),
    wonOpportunities: new Map(),
  };
  const organization = await findOneByName(
    client,
    'companies',
    update.organization.name,
    {},
  );
  plan.matches.set(update.organization, organization);

  const people = organization
    ? await client.list('people', {
        field: 'companyId',
        value: organization.id,
      })
    : [];
  for (const contact of update.contacts ?? []) {
    plan.matches.set(contact, findContact(people, contact));
  }

  const account =
    organization && update.account
      ? await findOneByName(
          client,
          'commercialAccounts',
          update.account.name,
          { organizationId: organization.id },
        )
      : undefined;
  if (update.account) plan.matches.set(update.account, account);

  for (const opportunity of update.opportunities ?? []) {
    const existing = organization
      ? await findOneByName(client, 'opportunities', opportunity.name, {
          companyId: organization.id,
        })
      : undefined;
    plan.matches.set(opportunity, existing);
  }

  for (const arrangement of update.arrangements ?? []) {
    const arrangementRecord = account
      ? await findOneByName(
          client,
          'commercialArrangements',
          arrangement.name,
          { accountId: account.id },
        )
      : undefined;
    plan.matches.set(arrangement, arrangementRecord);

    if (
      arrangement.wonOpportunity &&
      !(update.opportunities ?? []).some(
        (opportunity) => opportunity.name === arrangement.wonOpportunity,
      )
    ) {
      const wonOpportunity = organization
        ? await findOneByName(
            client,
            'opportunities',
            arrangement.wonOpportunity,
            { companyId: organization.id },
          )
        : undefined;
      if (!wonOpportunity) {
        throw new Error(
          `won opportunity ${JSON.stringify(arrangement.wonOpportunity)} was not found`,
        );
      }
      plan.wonOpportunities.set(arrangement, wonOpportunity);
    }

    for (const purchasedPackage of arrangement.packages ?? []) {
      const packageRecord = arrangementRecord
        ? await findPurchasedPackage(
            client,
            purchasedPackage,
            arrangementRecord.id,
          )
        : undefined;
      plan.matches.set(purchasedPackage, packageRecord);

      for (const line of purchasedPackage.offeringLines ?? []) {
        const lineRecord = packageRecord
          ? await findOneByName(client, 'offeringLines', line.name, {
              purchasedPackageId: packageRecord.id,
            })
          : undefined;
        plan.matches.set(line, lineRecord);
      }

      for (const charge of purchasedPackage.charges ?? []) {
        const chargeRecord =
          account && packageRecord
            ? await findOneByName(client, 'charges', charge.name, {
                accountId: account.id,
                purchasedPackageId: packageRecord.id,
              })
            : undefined;
        plan.matches.set(charge, chargeRecord);

        for (const payment of charge.payments ?? []) {
          const paymentRecord =
            account && chargeRecord
              ? await findOneByName(
                  client,
                  'incomingPayments',
                  payment.name,
                  {
                    payerAccountId: account.id,
                    chargeId: chargeRecord.id,
                  },
                )
              : undefined;
          plan.matches.set(payment, paymentRecord);
        }
      }
    }
  }

  return plan;
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

async function persistContact(
  client: TwentyClient,
  organization: TwentyRecord,
  contact: ContactUpdate,
  existing: TwentyRecord | undefined,
  changes: Change[],
): Promise<TwentyRecord> {
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
  if (existing) {
    const updated = await client.update('people', existing.id, data);
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

async function persistPlannedRecord(
  client: TwentyClient,
  resource: string,
  name: string,
  data: Record<string, unknown>,
  existing: TwentyRecord | undefined,
  changes: Change[],
): Promise<TwentyRecord> {
  if (existing) {
    const updated = await client.update(resource, existing.id, data);
    changes.push({ action: 'updated', resource, id: updated.id, name });
    return updated;
  }
  const created = await client.create(resource, data);
  changes.push({ action: 'created', resource, id: created.id, name });
  return created;
}

async function findPurchasedPackage(
  client: TwentyClient,
  purchasedPackage: PurchasedPackageUpdate,
  arrangementId: string,
): Promise<TwentyRecord | undefined> {
  const candidates = await client.list('purchasedPackages', {
    field: 'arrangementId',
    value: arrangementId,
  });
  const matching = candidates.filter((record) =>
    purchasedPackage.priceTermKey
      ? record.priceTermKey === purchasedPackage.priceTermKey
      : record.name === purchasedPackage.name && !record.priceTermKey,
  );
  if (matching.length > 1) {
    const identity = purchasedPackage.priceTermKey ?? purchasedPackage.name;
    throw new Error(
      `ambiguous purchasedPackages match for ${JSON.stringify(identity)}; no record was changed`,
    );
  }
  return matching[0];
}

function findContact(
  people: TwentyRecord[],
  contact: ContactUpdate,
): TwentyRecord | undefined {
  const normalizedEmail = contact.email?.toLowerCase();
  const matching = people.filter((person) => {
    if (normalizedEmail) {
      return contactEmail(person)?.toLowerCase() === normalizedEmail;
    }
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
  return matching[0];
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
  const explicit = resources.flatMap(([kind, records]) =>
    records
      .filter((record) => record.reconciliationWarning === true)
      .map((record) => `${kind}: ${String(record.name ?? record.id)}`),
  );
  const missingConversions = graph.incomingPayments
    .filter(
      (payment) =>
        payment.status === 'RECEIVED' &&
        typeof payment.assetCode === 'string' &&
        payment.assetCode.toUpperCase() !== 'USD' &&
        moneyAmount(payment.reportingValueUsd) === undefined,
    )
    .map(
      (payment) =>
        `payment: ${String(payment.name ?? payment.id)} lacks receipt-time USD value`,
    );
  return [...new Set([...explicit, ...missingConversions])];
}

function arrangementsForDisplay(
  graph: OrganizationGraph,
): Record<string, unknown>[] {
  return graph.arrangements
    .map((arrangement) => ({
      ...select(arrangement, [
        'id',
        'name',
        'status',
        'startsOn',
        'endsOn',
        'accountId',
        'wonOpportunityId',
        'sourceReference',
        'reconciliationWarning',
      ]),
      packages: graph.packages
        .filter((purchasedPackage) => purchasedPackage.arrangementId === arrangement.id)
        .map((purchasedPackage) => ({
          ...select(purchasedPackage, [
            'id',
            'name',
            'status',
            'priceBasis',
            'priceTermKey',
            'price',
            'billingCadence',
            'effectiveFrom',
            'effectiveTo',
            'monthlyRecurringRevenueUsd',
            'sourceReference',
            'reconciliationWarning',
          ]),
          offeringLines: graph.offeringLines
            .filter((line) => line.purchasedPackageId === purchasedPackage.id)
            .map((line) =>
              select(line, [
                'id',
                'name',
                'status',
                'fulfillmentPath',
                'quantity',
                'serviceStartsOn',
                'serviceEndsOn',
                'description',
              ]),
            )
            .sort(compareNames),
          charges: graph.charges
            .filter((charge) => charge.purchasedPackageId === purchasedPackage.id)
            .map((charge) => ({
              ...select(charge, [
                'id',
                'name',
                'amount',
                'status',
                'chargedOn',
                'dueOn',
                'accountId',
                'sourceReference',
                'reconciliationWarning',
              ]),
              payments: graph.incomingPayments
                .filter((payment) => payment.chargeId === charge.id)
                .map((payment) =>
                  select(payment, [
                    'id',
                    'name',
                    'nativeAmount',
                    'assetCode',
                    'network',
                    'reportingValueUsd',
                    'receivedAt',
                    'status',
                    'method',
                    'transactionReference',
                    'payerAccountId',
                    'sourceReference',
                    'reconciliationWarning',
                  ]),
                )
                .sort(compareNames),
            }))
            .sort(compareNames),
        }))
        .sort(compareNames),
    }))
    .sort(compareNames);
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
