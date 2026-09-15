import { defineField, FieldType, RelationType } from 'twenty-sdk/define';

import { COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-account.object';
import { COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-arrangement.object';
import {
  COMMERCIAL_ACCOUNT_ARRANGEMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  COMMERCIAL_ARRANGEMENT_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
} from './commercial-arrangement-account.field';

export default defineField({
  universalIdentifier:
    COMMERCIAL_ACCOUNT_ARRANGEMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'commercialArrangements',
  label: 'Commercial arrangements',
  icon: 'IconFileDescription',
  relationTargetObjectMetadataUniversalIdentifier:
    COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    COMMERCIAL_ARRANGEMENT_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
