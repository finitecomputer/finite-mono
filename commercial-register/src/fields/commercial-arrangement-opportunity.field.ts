import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-arrangement.object';

export const COMMERCIAL_ARRANGEMENT_OPPORTUNITY_FIELD_UNIVERSAL_IDENTIFIER =
  'e053cd29-5d01-4795-9e38-795e0cf17885';
export const OPPORTUNITY_COMMERCIAL_ARRANGEMENTS_FIELD_UNIVERSAL_IDENTIFIER =
  '1a7cd6ce-d690-4b5d-981f-5f6743311b3e';

export default defineField({
  universalIdentifier:
    COMMERCIAL_ARRANGEMENT_OPPORTUNITY_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'wonOpportunity',
  label: 'Won opportunity',
  description: 'Opportunity that resulted in this arrangement, when known',
  icon: 'IconTargetArrow',
  relationTargetObjectMetadataUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.universalIdentifier,
  relationTargetFieldMetadataUniversalIdentifier:
    OPPORTUNITY_COMMERCIAL_ARRANGEMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.SET_NULL,
    joinColumnName: 'wonOpportunityId',
  },
});
