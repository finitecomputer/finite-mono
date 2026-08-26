import { defineObject, FieldType } from 'twenty-sdk/define';

export const COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER =
  '47ca3bde-03e9-4429-b6c1-d38e7dd4a085';
export const COMMERCIAL_ARRANGEMENT_NAME_FIELD_UNIVERSAL_IDENTIFIER =
  'ab4d42ab-1bc0-4b68-a621-21bf066387eb';

export default defineObject({
  universalIdentifier: COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER,
  nameSingular: 'commercialArrangement',
  namePlural: 'commercialArrangements',
  labelSingular: 'Commercial arrangement',
  labelPlural: 'Commercial arrangements',
  description: 'Actual understanding under which Finite provides value',
  icon: 'IconFileDescription',
  isSearchable: true,
  labelIdentifierFieldMetadataUniversalIdentifier:
    COMMERCIAL_ARRANGEMENT_NAME_FIELD_UNIVERSAL_IDENTIFIER,
  fields: [
    {
      universalIdentifier:
        COMMERCIAL_ARRANGEMENT_NAME_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.TEXT,
      name: 'name',
      label: 'Name',
      icon: 'IconAbc',
    },
    {
      universalIdentifier: '47ee8fa6-c59d-4586-bf5e-c8eae91103be',
      type: FieldType.SELECT,
      name: 'status',
      label: 'Status',
      icon: 'IconActivity',
      defaultValue: "'ACTIVE'",
      options: [
        {
          id: '2a0a39b8-a817-4f6b-8c1a-baf8e551befd',
          value: 'ACTIVE',
          label: 'Active',
          position: 0,
          color: 'green',
        },
        {
          id: '84edab64-8f77-48e7-ae66-0e591e264089',
          value: 'COMPLETED',
          label: 'Completed',
          position: 1,
          color: 'blue',
        },
        {
          id: 'd4e11243-373f-4d15-9860-e4819e4356f9',
          value: 'CANCELLED',
          label: 'Cancelled',
          position: 2,
          color: 'gray',
        },
      ],
    },
    {
      universalIdentifier: 'ff8a7516-bf6d-45cb-af82-e5fafcda3438',
      type: FieldType.DATE,
      name: 'startsOn',
      label: 'Starts on',
      icon: 'IconCalendar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '6e1eb787-c457-4eca-8808-49c8b1e4c13c',
      type: FieldType.DATE,
      name: 'endsOn',
      label: 'Ends on',
      icon: 'IconCalendar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '7a47b9c3-68e5-4ca0-9b55-30e028f29192',
      type: FieldType.TEXT,
      name: 'sourceReference',
      label: 'Source reference',
      icon: 'IconLink',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: 'd3310972-cf1d-4967-8883-8c9152e3ec61',
      type: FieldType.BOOLEAN,
      name: 'reconciliationWarning',
      label: 'Reconciliation warning',
      icon: 'IconAlertCircle',
      defaultValue: false,
    },
  ],
});
