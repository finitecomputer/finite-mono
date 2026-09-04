import {
  defineField,
  FieldType,
  RelationType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-arrangement.object';
import {
  COMMERCIAL_ARRANGEMENT_OPPORTUNITY_FIELD_UNIVERSAL_IDENTIFIER,
  OPPORTUNITY_COMMERCIAL_ARRANGEMENTS_FIELD_UNIVERSAL_IDENTIFIER,
} from './commercial-arrangement-opportunity.field';

export default defineField({
  universalIdentifier:
    OPPORTUNITY_COMMERCIAL_ARRANGEMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.universalIdentifier,
  type: FieldType.RELATION,
  name: 'commercialArrangements',
  label: 'Commercial arrangements',
  icon: 'IconFileDescription',
  relationTargetObjectMetadataUniversalIdentifier:
    COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    COMMERCIAL_ARRANGEMENT_OPPORTUNITY_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
