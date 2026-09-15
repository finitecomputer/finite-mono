import {
  defineField,
  FieldType,
  RelationType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-account.object';
import {
  COMMERCIAL_ACCOUNT_ORGANIZATION_FIELD_UNIVERSAL_IDENTIFIER,
  COMPANY_COMMERCIAL_ACCOUNTS_FIELD_UNIVERSAL_IDENTIFIER,
} from './commercial-account-organization.field';

export default defineField({
  universalIdentifier:
    COMPANY_COMMERCIAL_ACCOUNTS_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.RELATION,
  name: 'commercialAccounts',
  label: 'Commercial accounts',
  description: 'Billing and service-ownership accounts for this organization',
  icon: 'IconBuildingBank',
  relationTargetObjectMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_ORGANIZATION_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
