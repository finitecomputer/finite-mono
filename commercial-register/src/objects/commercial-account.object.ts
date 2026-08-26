import { defineObject, FieldType } from 'twenty-sdk/define';

export const COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER =
  '341b3c7e-0144-43ba-b64c-65d8f9bd5f7a';
export const COMMERCIAL_ACCOUNT_NAME_FIELD_UNIVERSAL_IDENTIFIER =
  'be17140d-2f13-465d-9f0a-46612cf7753c';
export const COMMERCIAL_ACCOUNT_STATUS_FIELD_UNIVERSAL_IDENTIFIER =
  '9a48ab90-9053-47f4-b1cf-3969e2d83009';

export default defineObject({
  universalIdentifier: COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  nameSingular: 'commercialAccount',
  namePlural: 'commercialAccounts',
  labelSingular: 'Commercial account',
  labelPlural: 'Commercial accounts',
  description: 'Billing and service-ownership entity for one organization',
  icon: 'IconBuildingBank',
  isSearchable: true,
  labelIdentifierFieldMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_NAME_FIELD_UNIVERSAL_IDENTIFIER,
  fields: [
    {
      universalIdentifier:
        COMMERCIAL_ACCOUNT_NAME_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.TEXT,
      name: 'name',
      label: 'Name',
      description: 'Human-readable account name',
      icon: 'IconAbc',
    },
    {
      universalIdentifier:
        COMMERCIAL_ACCOUNT_STATUS_FIELD_UNIVERSAL_IDENTIFIER,
      type: FieldType.SELECT,
      name: 'status',
      label: 'Status',
      description: 'Whether the commercial account is active',
      icon: 'IconActivity',
      defaultValue: "'ACTIVE'",
      options: [
        {
          id: 'ac77ea03-3b7f-439c-a22e-da0195f45281',
          value: 'ACTIVE',
          label: 'Active',
          position: 0,
          color: 'green',
        },
        {
          id: '2500d874-5b3d-44c2-be21-dc39aa493be2',
          value: 'INACTIVE',
          label: 'Inactive',
          position: 1,
          color: 'gray',
        },
      ],
    },
    {
      universalIdentifier: 'b5268ddb-bb91-450a-bbcb-296c27a6511b',
      type: FieldType.TEXT,
      name: 'sourceReference',
      label: 'Source reference',
      description: 'Optional evidence pointer for the account record',
      icon: 'IconLink',
      isNullable: true,
      defaultValue: null,
    },
    {
      universalIdentifier: '7c26d313-1780-42cf-acf6-1a6ef89ea7cb',
      type: FieldType.BOOLEAN,
      name: 'reconciliationWarning',
      label: 'Reconciliation warning',
      description: 'Marks an incomplete or ambiguous account fact',
      icon: 'IconAlertCircle',
      defaultValue: false,
    },
  ],
});
