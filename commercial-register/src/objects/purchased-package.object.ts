import { defineObject, FieldType } from 'twenty-sdk/define';

export const PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER =
  '664e2beb-6b4e-4e2b-9601-018ffbaeb257';
export const PURCHASED_PACKAGE_NAME_FIELD_UNIVERSAL_IDENTIFIER =
  'da9d228d-36dd-41c7-8eb3-1172c55e6f01';
export const PURCHASED_PACKAGE_STATUS_FIELD_UNIVERSAL_IDENTIFIER =
  '3c37eff3-b243-44fc-82d9-6ac61f6a0407';
export const PURCHASED_PACKAGE_PRICE_BASIS_FIELD_UNIVERSAL_IDENTIFIER =
  'e2130067-ac07-4780-9c19-48cc6bc31ff3';
export const PURCHASED_PACKAGE_PRICE_FIELD_UNIVERSAL_IDENTIFIER =
  '987175da-131a-4dde-8952-663026008fab';
export const PURCHASED_PACKAGE_BILLING_CADENCE_FIELD_UNIVERSAL_IDENTIFIER =
  '020f81a1-d55c-4590-b478-7b0b2bfa78e3';
export const PURCHASED_PACKAGE_MRR_FIELD_UNIVERSAL_IDENTIFIER =
  'b74071c0-e0a7-4b54-87d8-fc411fca5306';
export const PURCHASED_PACKAGE_SOURCED_MONTHLY_PRICE_USD_FIELD_UNIVERSAL_IDENTIFIER =
  '8d9c485e-5185-4c8b-a90d-df43cbab276f';
export const PURCHASED_PACKAGE_PRICE_TERM_KEY_FIELD_UNIVERSAL_IDENTIFIER =
  'ec52a3f2-2e99-4822-879c-0f62494a5542';

export default defineObject({
  universalIdentifier: PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER,
  nameSingular: 'purchasedPackage',
  namePlural: 'purchasedPackages',
  labelSingular: 'Purchased package',
  labelPlural: 'Purchased packages',
  description: 'Offering lines purchased together under shared terms',
  icon: 'IconPackage',
  isSearchable: true,
  labelIdentifierFieldMetadataUniversalIdentifier:
    PURCHASED_PACKAGE_NAME_FIELD_UNIVERSAL_IDENTIFIER,
  fields: [
    {
      universalIdentifier:
        PURCHASED_PACKAGE_NAME_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.TEXT,
      name: 'name',
      label: 'Name',
      icon: 'IconAbc',
    },
    {
      universalIdentifier:
        PURCHASED_PACKAGE_STATUS_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.SELECT,
      name: 'status',
      label: 'Status',
      icon: 'IconActivity',
      defaultValue: "'PLANNED'",
      options: [
        {
          id: '67e0ec2c-62c7-4a0e-94c9-9c03aecfd088',
          value: 'PLANNED',
          label: 'Planned',
          position: 0,
          color: 'gray',
        },
        {
          id: 'fb859ee2-0584-49d1-8c48-898efb9a52eb',
          value: 'ACTIVE',
          label: 'Active',
          position: 1,
          color: 'green',
        },
        {
          id: '39b222bb-5ffb-4eb6-ac94-1a247a9804a0',
          value: 'COMPLETED',
          label: 'Completed',
          position: 2,
          color: 'blue',
        },
        {
          id: 'f3a92d75-9e90-42df-9d87-97cf7b1cd5f8',
          value: 'CANCELLED',
          label: 'Cancelled',
          position: 3,
          color: 'gray',
        },
      ],
    },
    {
      universalIdentifier:
        PURCHASED_PACKAGE_PRICE_TERM_KEY_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.TEXT,
      name: 'priceTermKey',
      label: 'Price term key',
      description:
        'Stable agent-supplied identity for one effective-dated recurring price term',
      icon: 'IconKey',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier:
        PURCHASED_PACKAGE_PRICE_BASIS_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.SELECT,
      name: 'priceBasis',
      label: 'Price basis',
      icon: 'IconReceipt',
      defaultValue: "'ONE_TIME'",
      options: [
        {
          id: 'e6579b03-d279-4694-866b-216dffc41e41',
          value: 'ONE_TIME',
          label: 'One time',
          position: 0,
          color: 'blue',
        },
        {
          id: '91e8a39a-edcb-48ae-a5af-8e707b639ba5',
          value: 'RECURRING',
          label: 'Recurring',
          position: 1,
          color: 'green',
        },
        {
          id: '1845cbc8-3840-4e37-ab45-e83d2b58464a',
          value: 'USAGE',
          label: 'Usage',
          position: 2,
          color: 'orange',
        },
        {
          id: '48439245-f0cd-4c64-bf3e-8dc6206709a8',
          value: 'INCLUDED',
          label: 'Included',
          position: 3,
          color: 'gray',
        },
      ],
    },
    {
      universalIdentifier: PURCHASED_PACKAGE_PRICE_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.CURRENCY,
      name: 'price',
      label: 'Agreed price',
      icon: 'IconCurrencyDollar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier:
        PURCHASED_PACKAGE_BILLING_CADENCE_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.SELECT,
      name: 'billingCadence',
      label: 'Billing cadence',
      icon: 'IconRepeat',
      defaultValue: "'NONE'",
      options: [
        {
          id: '35412d60-59fd-4070-a3b0-12d1a8f64a5e',
          value: 'NONE',
          label: 'None',
          position: 0,
          color: 'gray',
        },
        {
          id: '07238c30-87b4-47a0-a71d-f8b6a53551ce',
          value: 'MONTHLY',
          label: 'Monthly',
          position: 1,
          color: 'green',
        },
        {
          id: 'd750c274-51e0-4fb5-86dd-56c45c3dfcbe',
          value: 'QUARTERLY',
          label: 'Quarterly',
          position: 2,
          color: 'blue',
        },
        {
          id: '9e6fc154-3cea-4d46-84c6-6260bdf701c8',
          value: 'ANNUAL',
          label: 'Annual',
          position: 3,
          color: 'purple',
        },
      ],
    },
    {
      universalIdentifier: '3b0df5b7-219c-4b91-962e-6b2eb31f46da',
      type: FieldType.DATE,
      name: 'effectiveFrom',
      label: 'Effective from',
      icon: 'IconCalendar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '49368ff7-6257-4a3d-a65c-d1a14160b94b',
      type: FieldType.DATE,
      name: 'effectiveTo',
      label: 'Effective to',
      icon: 'IconCalendar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier:
        PURCHASED_PACKAGE_SOURCED_MONTHLY_PRICE_USD_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.CURRENCY,
      name: 'sourcedMonthlyPriceUsd',
      label: 'Sourced monthly price (USD)',
      description:
        'Source fact used to normalize a non-USD recurring price; not a calculated projection',
      icon: 'IconCurrencyDollar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: PURCHASED_PACKAGE_MRR_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.CURRENCY,
      name: 'monthlyRecurringRevenueUsd',
      label: 'Monthly recurring revenue (derived)',
      description: 'USD monthly value derived from this package price term',
      icon: 'IconRepeat',
      isNullable: true,
      defaultValue: null,
      isUIEditable: false,
    },
    {
      universalIdentifier: '62f4957d-75f4-400f-ac25-775968ead716',
      type: FieldType.TEXT,
      name: 'sourceReference',
      label: 'Source reference',
      icon: 'IconLink',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: 'f85e1a3a-6ca1-4c1e-907c-5dd01ff880f1',
      type: FieldType.BOOLEAN,
      name: 'reconciliationWarning',
      label: 'Reconciliation warning',
      icon: 'IconAlertCircle',
      defaultValue: false,
    },
  ],
});
