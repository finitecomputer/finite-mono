import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-account.object';

export const COMMERCIAL_ACCOUNT_ORGANIZATION_FIELD_UNIVERSAL_IDENTIFIER =
  '81358779-63a0-4648-9b40-500e23101055';
export const COMPANY_COMMERCIAL_ACCOUNTS_FIELD_UNIVERSAL_IDENTIFIER =
  'a31f2864-6b22-44e5-973a-1ae9e1b2c130';

export default defineField({
  universalIdentifier:
    COMMERCIAL_ACCOUNT_ORGANIZATION_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'organization',
  label: 'Organization',
  description: 'Organization that owns this commercial account',
  icon: 'IconBuilding',
  relationTargetObjectMetadataUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  relationTargetFieldMetadataUniversalIdentifier:
    COMPANY_COMMERCIAL_ACCOUNTS_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.RESTRICT,
    joinColumnName: 'organizationId',
  },
});
