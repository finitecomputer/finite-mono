import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
} from 'twenty-sdk/define';

import { CHARGE_UNIVERSAL_IDENTIFIER } from '../objects/charge.object';
import { COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-account.object';

export const CHARGE_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER =
  '855cb828-c181-46cc-bb8a-c5963a75c54b';
export const COMMERCIAL_ACCOUNT_CHARGES_FIELD_UNIVERSAL_IDENTIFIER =
  'c11dbb1b-201a-4d27-96f7-63f24dc39493';

export default defineField({
  universalIdentifier: CHARGE_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: CHARGE_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'account',
  label: 'Account expected to pay',
  icon: 'IconBuildingBank',
  relationTargetObjectMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_CHARGES_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.RESTRICT,
    joinColumnName: 'accountId',
  },
});
