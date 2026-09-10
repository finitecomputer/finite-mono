export type CommercialRole =
  | 'PROSPECT'
  | 'CUSTOMER'
  | 'SPONSOR'
  | 'PARTNER'
  | 'FORMER_CUSTOMER';

export type OpportunityStage =
  | 'EXPLORING'
  | 'PROPOSAL_DRAFTED'
  | 'PROPOSAL_SENT'
  | 'WON'
  | 'LOST'
  | 'PAUSED';

export interface MoneyInput {
  amount: number;
  currencyCode: string;
}

export interface OrganizationUpdate {
  name: string;
  domainName?: string;
  commercialRoles?: CommercialRole[];
  brainPage?: string;
  relationshipSummary?: string;
  relationshipSummaryRefreshedAt?: string;
  sourceReference?: string;
  reconciliationWarning?: boolean;
}

export interface AccountUpdate {
  name: string;
  status?: 'ACTIVE' | 'INACTIVE';
  cashHistoryReconciled?: boolean;
  sourceReference?: string;
  reconciliationWarning?: boolean;
}

export interface ContactUpdate {
  firstName: string;
  lastName: string;
  email?: string;
  jobTitle?: string;
  linkedinUrl?: string;
}

export interface OpportunityUpdate {
  name: string;
  stage: OpportunityStage;
  amount?: MoneyInput;
  brainWants?: string;
  sourceReference?: string;
  reconciliationWarning?: boolean;
}

export interface OfferingLineUpdate {
  name: string;
  status?: 'PLANNED' | 'ACTIVE' | 'COMPLETED' | 'CANCELLED';
  fulfillmentPath?:
    | 'IN_PERSON'
    | 'FIRST_CLASS_PLATFORM'
    | 'LEGACY_SYSTEM'
    | 'EXTERNAL'
    | 'OTHER';
  quantity?: number;
  serviceStartsOn?: string;
  serviceEndsOn?: string;
  description?: string;
}

export interface IncomingPaymentUpdate {
  name: string;
  nativeAmount: number;
  assetCode: string;
  reportingValueUsd?: number;
  network?: string;
  receivedAt: string;
  status?: 'RECEIVED' | 'REFUNDED' | 'VOIDED';
  method?: 'BANK' | 'CARD' | 'DIGITAL_ASSET' | 'CASH' | 'OTHER';
  transactionReference?: string;
  sourceReference?: string;
  reconciliationWarning?: boolean;
}

export interface ChargeUpdate {
  name: string;
  amount: MoneyInput;
  status?: 'OPEN' | 'PAID' | 'VOID';
  chargedOn?: string;
  dueOn?: string;
  sourceReference?: string;
  reconciliationWarning?: boolean;
  payments?: IncomingPaymentUpdate[];
}

export interface PurchasedPackageUpdate {
  name: string;
  status?: 'PLANNED' | 'ACTIVE' | 'COMPLETED' | 'CANCELLED';
  priceBasis?: 'ONE_TIME' | 'RECURRING' | 'USAGE' | 'INCLUDED';
  priceTermKey?: string;
  price?: MoneyInput;
  billingCadence?: 'NONE' | 'MONTHLY' | 'QUARTERLY' | 'ANNUAL';
  effectiveFrom?: string;
  effectiveTo?: string;
  sourcedMonthlyPriceUsd?: number;
  sourceReference?: string;
  reconciliationWarning?: boolean;
  offeringLines?: OfferingLineUpdate[];
  charges?: ChargeUpdate[];
}

export interface CommercialArrangementUpdate {
  name: string;
  status?: 'ACTIVE' | 'COMPLETED' | 'CANCELLED';
  startsOn?: string;
  endsOn?: string;
  wonOpportunity?: string;
  sourceReference?: string;
  reconciliationWarning?: boolean;
  packages?: PurchasedPackageUpdate[];
}

export interface CommercialUpdate {
  version: 1;
  organization: OrganizationUpdate;
  account?: AccountUpdate;
  contacts?: ContactUpdate[];
  opportunities?: OpportunityUpdate[];
  arrangements?: CommercialArrangementUpdate[];
}

export type TwentyRecord = Record<string, unknown> & { id: string };

export interface CommercialMetrics {
  currentMrrUsd: number;
  lifetimeNetCashUsd: number | null;
  isCurrentCustomer: boolean;
}
