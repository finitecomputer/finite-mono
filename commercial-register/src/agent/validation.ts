import type {
  CommercialArrangementUpdate,
  CommercialUpdate,
  IncomingPaymentUpdate,
  PurchasedPackageUpdate,
} from './types';

export function parseCommercialUpdate(value: unknown): CommercialUpdate {
  const update = objectAt(value, 'update');
  if (update.version !== 1) {
    throw new Error('update.version must be 1');
  }

  const organization = objectAt(update.organization, 'organization');
  nonEmptyString(organization.name, 'organization.name');
  optionalString(organization.domainName, 'organization.domainName');
  optionalString(organization.brainPage, 'organization.brainPage');
  optionalString(
    organization.relationshipSummary,
    'organization.relationshipSummary',
  );
  optionalDateTime(
    organization.relationshipSummaryRefreshedAt,
    'organization.relationshipSummaryRefreshedAt',
  );
  if (
    organization.relationshipSummary !== undefined &&
    organization.relationshipSummaryRefreshedAt === undefined
  ) {
    throw new Error(
      'organization.relationshipSummaryRefreshedAt is required with relationshipSummary',
    );
  }
  optionalString(organization.sourceReference, 'organization.sourceReference');
  optionalBoolean(
    organization.reconciliationWarning,
    'organization.reconciliationWarning',
  );
  const commercialRoles = optionalArray(
    organization.commercialRoles,
    'organization.commercialRoles',
  );
  for (const [index, role] of commercialRoles.entries()) {
    enumValue(
      role,
      ['PROSPECT', 'CUSTOMER', 'SPONSOR', 'PARTNER', 'FORMER_CUSTOMER'],
      `organization.commercialRoles[${index}]`,
    );
  }

  if (update.account !== undefined) {
    const account = objectAt(update.account, 'account');
    nonEmptyString(account.name, 'account.name');
    optionalEnum(account.status, ['ACTIVE', 'INACTIVE'], 'account.status');
    optionalString(account.sourceReference, 'account.sourceReference');
    optionalBoolean(
      account.reconciliationWarning,
      'account.reconciliationWarning',
    );
  }

  const contacts = optionalArray(update.contacts, 'contacts');
  const contactKeys: string[] = [];
  for (const [index, rawContact] of contacts.entries()) {
    const contact = objectAt(rawContact, `contacts[${index}]`);
    nonEmptyString(contact.firstName, `contacts[${index}].firstName`);
    nonEmptyString(contact.lastName, `contacts[${index}].lastName`);
    optionalString(contact.email, `contacts[${index}].email`);
    optionalString(contact.jobTitle, `contacts[${index}].jobTitle`);
    optionalString(contact.linkedinUrl, `contacts[${index}].linkedinUrl`);
    const key =
      typeof contact.email === 'string'
        ? contact.email.toLowerCase()
        : `${String(contact.firstName).toLowerCase()} ${String(contact.lastName).toLowerCase()}`;
    if (contactKeys.includes(key)) {
      throw new Error(`contacts contains duplicate ${JSON.stringify(key)}`);
    }
    contactKeys.push(key);
  }

  const opportunities = optionalArray(update.opportunities, 'opportunities');
  uniqueNames(
    opportunities as Array<{ name?: unknown }>,
    'opportunities',
  );
  for (const [index, rawOpportunity] of opportunities.entries()) {
    const opportunity = objectAt(rawOpportunity, `opportunities[${index}]`);
    nonEmptyString(opportunity.name, `opportunities[${index}].name`);
    enumValue(
      opportunity.stage,
      [
        'EXPLORING',
        'PROPOSAL_DRAFTED',
        'PROPOSAL_SENT',
        'WON',
        'LOST',
        'PAUSED',
      ],
      `opportunities[${index}].stage`,
    );
    optionalString(
      opportunity.sourceReference,
      `opportunities[${index}].sourceReference`,
    );
    optionalBoolean(
      opportunity.reconciliationWarning,
      `opportunities[${index}].reconciliationWarning`,
    );
  }

  const arrangements = optionalArray(update.arrangements, 'arrangements');
  uniqueNames(
    arrangements as Array<{ name?: unknown }>,
    'arrangements',
  );
  if (arrangements.length > 0 && update.account === undefined) {
    throw new Error('account is required when arrangements are supplied');
  }
  for (const [index, rawArrangement] of arrangements.entries()) {
    validateArrangement(
      objectAt(rawArrangement, `arrangements[${index}]`) as unknown as CommercialArrangementUpdate,
      `arrangements[${index}]`,
    );
  }

  return update as unknown as CommercialUpdate;
}

function validateArrangement(
  arrangement: CommercialArrangementUpdate,
  path: string,
): void {
  nonEmptyString(arrangement.name, `${path}.name`);
  optionalEnum(
    arrangement.status,
    ['ACTIVE', 'COMPLETED', 'CANCELLED'],
    `${path}.status`,
  );
  optionalDate(arrangement.startsOn, `${path}.startsOn`);
  optionalDate(arrangement.endsOn, `${path}.endsOn`);
  optionalString(arrangement.sourceReference, `${path}.sourceReference`);
  optionalBoolean(
    arrangement.reconciliationWarning,
    `${path}.reconciliationWarning`,
  );
  uniquePackageIdentities(arrangement.packages ?? [], `${path}.packages`);
  uniquePriceTermKeys(arrangement.packages ?? [], `${path}.packages`);
  for (const [index, purchasedPackage] of (arrangement.packages ?? []).entries()) {
    validatePackage(purchasedPackage, `${path}.packages[${index}]`);
  }
}

function validatePackage(
  purchasedPackage: PurchasedPackageUpdate,
  path: string,
): void {
  nonEmptyString(purchasedPackage.name, `${path}.name`);
  optionalEnum(
    purchasedPackage.status,
    ['PLANNED', 'ACTIVE', 'COMPLETED', 'CANCELLED'],
    `${path}.status`,
  );
  optionalEnum(
    purchasedPackage.priceBasis,
    ['ONE_TIME', 'RECURRING', 'USAGE', 'INCLUDED'],
    `${path}.priceBasis`,
  );
  optionalEnum(
    purchasedPackage.billingCadence,
    ['NONE', 'MONTHLY', 'QUARTERLY', 'ANNUAL'],
    `${path}.billingCadence`,
  );
  optionalDate(purchasedPackage.effectiveFrom, `${path}.effectiveFrom`);
  optionalDate(purchasedPackage.effectiveTo, `${path}.effectiveTo`);
  optionalString(purchasedPackage.priceTermKey, `${path}.priceTermKey`);
  optionalString(purchasedPackage.sourceReference, `${path}.sourceReference`);
  optionalBoolean(
    purchasedPackage.reconciliationWarning,
    `${path}.reconciliationWarning`,
  );
  if (purchasedPackage.price !== undefined) {
    positiveMoney(purchasedPackage.price, `${path}.price`);
    evidenceOrWarning(purchasedPackage, path);
  }
  if (
    purchasedPackage.monthlyRecurringRevenueUsd !== undefined &&
    (!Number.isFinite(purchasedPackage.monthlyRecurringRevenueUsd) ||
      purchasedPackage.monthlyRecurringRevenueUsd < 0)
  ) {
    throw new Error(`${path}.monthlyRecurringRevenueUsd must not be negative`);
  }
  if (purchasedPackage.priceBasis === 'RECURRING') {
    if (purchasedPackage.price === undefined) {
      throw new Error(`${path}.price is required for a recurring package`);
    }
    if (
      purchasedPackage.billingCadence === undefined ||
      purchasedPackage.billingCadence === 'NONE'
    ) {
      throw new Error(
        `${path}.billingCadence must be monthly, quarterly, or annual for a recurring package`,
      );
    }
    nonEmptyString(purchasedPackage.effectiveFrom, `${path}.effectiveFrom`);
    nonEmptyString(purchasedPackage.priceTermKey, `${path}.priceTermKey`);
    if (
      purchasedPackage.price.currencyCode.toUpperCase() !== 'USD' &&
      purchasedPackage.monthlyRecurringRevenueUsd === undefined
    ) {
      throw new Error(
        `${path}.monthlyRecurringRevenueUsd is required for a non-USD recurring package`,
      );
    }
  }

  uniqueNames(purchasedPackage.offeringLines ?? [], `${path}.offeringLines`);
  for (const [index, line] of (purchasedPackage.offeringLines ?? []).entries()) {
    nonEmptyString(line.name, `${path}.offeringLines[${index}].name`);
    optionalEnum(
      line.status,
      ['PLANNED', 'ACTIVE', 'COMPLETED', 'CANCELLED'],
      `${path}.offeringLines[${index}].status`,
    );
    optionalEnum(
      line.fulfillmentPath,
      [
        'IN_PERSON',
        'FIRST_CLASS_PLATFORM',
        'LEGACY_SYSTEM',
        'EXTERNAL',
        'OTHER',
      ],
      `${path}.offeringLines[${index}].fulfillmentPath`,
    );
    if (
      line.quantity !== undefined &&
      (!Number.isFinite(line.quantity) || line.quantity < 0)
    ) {
      throw new Error(
        `${path}.offeringLines[${index}].quantity must not be negative`,
      );
    }
    optionalDate(
      line.serviceStartsOn,
      `${path}.offeringLines[${index}].serviceStartsOn`,
    );
    optionalDate(
      line.serviceEndsOn,
      `${path}.offeringLines[${index}].serviceEndsOn`,
    );
  }
  uniqueNames(purchasedPackage.charges ?? [], `${path}.charges`);
  for (const [index, charge] of (purchasedPackage.charges ?? []).entries()) {
    const chargePath = `${path}.charges[${index}]`;
    nonEmptyString(charge.name, `${chargePath}.name`);
    positiveMoney(charge.amount, `${chargePath}.amount`);
    optionalEnum(charge.status, ['OPEN', 'PAID', 'VOID'], `${chargePath}.status`);
    optionalDate(charge.chargedOn, `${chargePath}.chargedOn`);
    optionalDate(charge.dueOn, `${chargePath}.dueOn`);
    optionalString(charge.sourceReference, `${chargePath}.sourceReference`);
    optionalBoolean(
      charge.reconciliationWarning,
      `${chargePath}.reconciliationWarning`,
    );
    evidenceOrWarning(charge, chargePath);
    uniqueNames(charge.payments ?? [], `${chargePath}.payments`);
    for (const [paymentIndex, payment] of (charge.payments ?? []).entries()) {
      validatePayment(payment, `${chargePath}.payments[${paymentIndex}]`);
    }
  }
}

function validatePayment(payment: IncomingPaymentUpdate, path: string): void {
  nonEmptyString(payment.name, `${path}.name`);
  if (!Number.isFinite(payment.nativeAmount) || payment.nativeAmount <= 0) {
    throw new Error(`${path}.nativeAmount must be a positive number`);
  }
  nonEmptyString(payment.assetCode, `${path}.assetCode`);
  nonEmptyString(payment.receivedAt, `${path}.receivedAt`);
  if (Number.isNaN(Date.parse(payment.receivedAt))) {
    throw new Error(`${path}.receivedAt must be an ISO date-time`);
  }
  optionalEnum(
    payment.status,
    ['RECEIVED', 'REFUNDED', 'VOIDED'],
    `${path}.status`,
  );
  optionalString(payment.network, `${path}.network`);
  optionalString(payment.sourceReference, `${path}.sourceReference`);
  optionalBoolean(
    payment.reconciliationWarning,
    `${path}.reconciliationWarning`,
  );
  evidenceOrWarning(payment, path);
  optionalEnum(
    payment.method,
    ['BANK', 'CARD', 'DIGITAL_ASSET', 'CASH', 'OTHER'],
    `${path}.method`,
  );
  if (
    payment.reportingValueUsd !== undefined &&
    (!Number.isFinite(payment.reportingValueUsd) ||
      payment.reportingValueUsd < 0)
  ) {
    throw new Error(`${path}.reportingValueUsd must not be negative`);
  }
}

function positiveMoney(value: unknown, path: string): void {
  const money = objectAt(value, path);
  if (typeof money.amount !== 'number' || !Number.isFinite(money.amount)) {
    throw new Error(`${path}.amount must be a number`);
  }
  if (money.amount < 0) throw new Error(`${path}.amount must not be negative`);
  nonEmptyString(money.currencyCode, `${path}.currencyCode`);
}

function objectAt(value: unknown, path: string): Record<string, unknown> {
  if (typeof value !== 'object' || value === null || Array.isArray(value)) {
    throw new Error(`${path} must be an object`);
  }
  return value as Record<string, unknown>;
}

function optionalArray(value: unknown, path: string): unknown[] {
  if (value === undefined) return [];
  if (!Array.isArray(value)) throw new Error(`${path} must be an array`);
  return value;
}

function nonEmptyString(value: unknown, path: string): asserts value is string {
  if (typeof value !== 'string' || value.trim() === '') {
    throw new Error(`${path} must be a non-empty string`);
  }
}

function optionalString(value: unknown, path: string): void {
  if (value !== undefined) nonEmptyString(value, path);
}

function optionalDate(value: unknown, path: string): void {
  if (value === undefined) return;
  nonEmptyString(value, path);
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value) || Number.isNaN(Date.parse(value))) {
    throw new Error(`${path} must be an ISO date (YYYY-MM-DD)`);
  }
}

function optionalDateTime(value: unknown, path: string): void {
  if (value === undefined) return;
  nonEmptyString(value, path);
  if (Number.isNaN(Date.parse(value))) {
    throw new Error(`${path} must be an ISO date-time`);
  }
}

function enumValue(
  value: unknown,
  allowed: readonly string[],
  path: string,
): void {
  if (typeof value !== 'string' || !allowed.includes(value)) {
    throw new Error(`${path} must be one of ${allowed.join(', ')}`);
  }
}

function optionalEnum(
  value: unknown,
  allowed: readonly string[],
  path: string,
): void {
  if (value !== undefined) enumValue(value, allowed, path);
}

function optionalBoolean(value: unknown, path: string): void {
  if (value !== undefined && typeof value !== 'boolean') {
    throw new Error(`${path} must be a boolean`);
  }
}

function evidenceOrWarning(
  value: { sourceReference?: unknown; reconciliationWarning?: unknown },
  path: string,
): void {
  const hasSource =
    typeof value.sourceReference === 'string' &&
    value.sourceReference.trim() !== '';
  if (!hasSource && value.reconciliationWarning !== true) {
    throw new Error(
      `${path} requires sourceReference or reconciliationWarning: true`,
    );
  }
}

function uniqueNames(
  values: Array<{ name?: unknown }>,
  path: string,
): void {
  const names = values.map((value, index) =>
    nonEmptyName(value.name, `${path}[${index}].name`),
  );
  const duplicate = names.find((name, index) => names.indexOf(name) !== index);
  if (duplicate) {
    throw new Error(`${path} contains duplicate name ${JSON.stringify(duplicate)}`);
  }
}

function uniquePriceTermKeys(
  values: PurchasedPackageUpdate[],
  path: string,
): void {
  const keys = values
    .map((value) => value.priceTermKey)
    .filter((value): value is string => value !== undefined);
  const duplicate = keys.find((key, index) => keys.indexOf(key) !== index);
  if (duplicate) {
    throw new Error(
      `${path} contains duplicate priceTermKey ${JSON.stringify(duplicate)}`,
    );
  }
}

function uniquePackageIdentities(
  values: PurchasedPackageUpdate[],
  path: string,
): void {
  const identities = values.map((value, index) => {
    nonEmptyString(value.name, `${path}[${index}].name`);
    return value.priceTermKey
      ? `price term ${value.priceTermKey}`
      : `package ${value.name}`;
  });
  const duplicate = identities.find(
    (identity, index) => identities.indexOf(identity) !== index,
  );
  if (duplicate) {
    throw new Error(`${path} contains duplicate ${JSON.stringify(duplicate)}`);
  }
}

function nonEmptyName(value: unknown, path: string): string {
  nonEmptyString(value, path);
  return value;
}
