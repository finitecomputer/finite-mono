import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
} from 'twenty-sdk/define';

import { COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-account.object';
import { COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-arrangement.object';

export const COMMERCIAL_ARRANGEMENT_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER =
  '04fbcccd-decc-4bef-a656-f6ca84a1dad1';
export const COMMERCIAL_ACCOUNT_ARRANGEMENTS_FIELD_UNIVERSAL_IDENTIFIER =
  'c8aaf253-1534-495a-b496-0ad1e8ad87cf';

export default defineField({
  universalIdentifier:
    COMMERCIAL_ARRANGEMENT_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'account',
  label: 'Account',
  description: 'Account that owns the purchased services',
  icon: 'IconBuildingBank',
  relationTargetObjectMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_ARRANGEMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.RESTRICT,
    joinColumnName: 'accountId',
  },
});
