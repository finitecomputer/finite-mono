import { defineObject, FieldType } from 'twenty-sdk/define';

export const OFFERING_LINE_UNIVERSAL_IDENTIFIER =
  '4ec09171-292c-4fd2-9b6a-2d80d470ad71';
export const OFFERING_LINE_NAME_FIELD_UNIVERSAL_IDENTIFIER =
  '8382230e-9065-445a-a1b6-c762e55df19e';

export default defineObject({
  universalIdentifier: OFFERING_LINE_UNIVERSAL_IDENTIFIER,
  nameSingular: 'offeringLine',
  namePlural: 'offeringLines',
  labelSingular: 'Offering line',
  labelPlural: 'Offering lines',
  description: 'One promised or purchased item within a package',
  icon: 'IconListDetails',
  isSearchable: true,
  labelIdentifierFieldMetadataUniversalIdentifier:
    OFFERING_LINE_NAME_FIELD_UNIVERSAL_IDENTIFIER,
  fields: [
    {
      universalIdentifier: OFFERING_LINE_NAME_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.TEXT,
      name: 'name',
      label: 'Offering',
      description: 'Actual offering name; a formal catalog is deferred',
      icon: 'IconAbc',
    },
    {
      universalIdentifier: '9c7f1da9-4593-4296-a1f7-6f400aa51fd4',
      type: FieldType.SELECT,
      name: 'status',
      label: 'Delivery status',
      icon: 'IconActivity',
      defaultValue: "'PLANNED'",
      options: [
        {
          id: 'de5c59be-11e8-4fd5-a7ea-681d87ab1525',
          value: 'PLANNED',
          label: 'Planned',
          position: 0,
          color: 'gray',
        },
        {
          id: '0e003231-5153-47c3-9938-634c248faecb',
          value: 'ACTIVE',
          label: 'Active',
          position: 1,
          color: 'green',
        },
        {
          id: '20a3ad8f-29f7-4257-8130-13699f0ccd85',
          value: 'COMPLETED',
          label: 'Completed',
          position: 2,
          color: 'blue',
        },
        {
          id: '7ecf5842-a778-4b42-9414-e61f36e3413d',
          value: 'CANCELLED',
          label: 'Cancelled',
          position: 3,
          color: 'gray',
        },
      ],
    },
    {
      universalIdentifier: '387b482c-5b0f-4b78-936b-5f4ae6675a21',
      type: FieldType.SELECT,
      name: 'fulfillmentPath',
      label: 'Fulfillment path',
      icon: 'IconRoute',
      defaultValue: "'OTHER'",
      options: [
        {
          id: '694f9e0e-7e89-43bc-abaf-016d4fcb6ab9',
          value: 'IN_PERSON',
          label: 'In person',
          position: 0,
          color: 'orange',
        },
        {
          id: '16bca3e3-34f9-4220-ad79-11ed94edd89b',
          value: 'FIRST_CLASS_PLATFORM',
          label: 'First-class platform',
          position: 1,
          color: 'green',
        },
        {
          id: '840c5815-8f48-4373-995d-4cce896df6a0',
          value: 'LEGACY_SYSTEM',
          label: 'Legacy system',
          position: 2,
          color: 'gray',
        },
        {
          id: '304c145e-9d07-40a6-9aa7-5ce1ccb54abb',
          value: 'EXTERNAL',
          label: 'External',
          position: 3,
          color: 'blue',
        },
        {
          id: '09fc4eed-0eaa-4907-82bc-6e5debbb14e6',
          value: 'OTHER',
          label: 'Other',
          position: 4,
          color: 'gray',
        },
      ],
    },
    {
      universalIdentifier: '8664ab1c-47e5-41c7-b84c-5b528835d2af',
      type: FieldType.NUMBER,
      name: 'quantity',
      label: 'Quantity',
      icon: 'IconNumbers',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '58cf4079-1138-4f80-95a4-a896719145ad',
      type: FieldType.DATE,
      name: 'serviceStartsOn',
      label: 'Service starts on',
      icon: 'IconCalendar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '504874cc-ca30-4254-aaeb-cb8aee53ec3d',
      type: FieldType.DATE,
      name: 'serviceEndsOn',
      label: 'Service ends on',
      icon: 'IconCalendar',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '84d750e1-ff8e-454b-8af7-45df8c3740c0',
      type: FieldType.TEXT,
      name: 'description',
      label: 'Description',
      description: 'Size, cohort, location, and other actual delivery terms',
      icon: 'IconNotes',
      isNullable: true,
      defaultValue: null,
    },
  ],
});
