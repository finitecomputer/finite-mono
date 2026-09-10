import { defineObject, FieldType } from 'twenty-sdk/define';

export const CHARGE_UNIVERSAL_IDENTIFIER =
  '4fffe80f-6b40-43c9-aa20-931581bffd7a';
export const CHARGE_NAME_FIELD_UNIVERSAL_IDENTIFIER =
  '8ab7533c-4d3e-4aa7-90c3-5e91dd071c8c';

export default defineObject({
  universalIdentifier: CHARGE_UNIVERSAL_IDENTIFIER,
  nameSingular: 'charge',
  namePlural: 'charges',
  labelSingular: 'Charge',
  labelPlural: 'Charges',
  description: 'An amount a commercial account is expected or requested to pay',
  icon: 'IconReceipt',
  isSearchable: true,
  labelIdentifierFieldMetadataUniversalIdentifier:
    CHARGE_NAME_FIELD_UNIVERSAL_IDENTIFIER,
  fields: [
    {
      universalIdentifier: CHARGE_NAME_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.TEXT,
      name: 'name',
      label: 'Name',
      icon: 'IconAbc',
    },
    {
      universalIdentifier: '039f08f7-1299-480f-9dd4-2fe858dcea2f',
      type: FieldType.CURRENCY,
      name: 'amount',
      label: 'Amount requested',
      icon: 'IconCurrencyDollar',
    },
    {
      universalIdentifier: 'bec710a1-5511-46fd-a69d-36f759808438',
      type: FieldType.SELECT,
      name: 'status',
      label: 'Status',
      icon: 'IconActivity',
      defaultValue: "'OPEN'",
      options: [
        {
          id: '7ab3f2ca-f658-4dea-8f65-b561f15a2dfc',
          value: 'OPEN',
          label: 'Open',
          position: 0,
          color: 'orange',
        },
        {
          id: '86f90e89-02e3-4f92-ad74-8a98c7533930',
          value: 'PAID',
          label: 'Paid',
          position: 1,
          color: 'green',
        },
        {
          id: '30cae497-5020-4951-8f26-4ad07a532f50',
          value: 'VOID',
          label: 'Void',
          position: 2,
          color: 'gray',
        },
      ],
    },
    {
      universalIdentifier: '4cfbed01-f49c-4f15-8d65-57175e78c204',
      type: FieldType.DATE,
      name: 'chargedOn',
      label: 'Charged on',
      icon: 'IconCalendar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '75c2b974-9776-47d7-a002-3f617ee63f5e',
      type: FieldType.DATE,
      name: 'dueOn',
      label: 'Due on',
      icon: 'IconCalendarDue',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '14af99da-fe6f-408a-9458-68337c9b125f',
      type: FieldType.TEXT,
      name: 'sourceReference',
      label: 'Source reference',
      icon: 'IconLink',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '74cba9bd-5f94-4809-98fd-6b096425a5ae',
      type: FieldType.BOOLEAN,
      name: 'reconciliationWarning',
      label: 'Reconciliation warning',
      icon: 'IconAlertCircle',
      defaultValue: false,
    },
  ],
});
