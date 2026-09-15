import { defineField, FieldType, RelationType } from 'twenty-sdk/define';

import { OFFERING_LINE_UNIVERSAL_IDENTIFIER } from '../objects/offering-line.object';
import { PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER } from '../objects/purchased-package.object';
import {
  OFFERING_LINE_PACKAGE_FIELD_UNIVERSAL_IDENTIFIER,
  PURCHASED_PACKAGE_OFFERING_LINES_FIELD_UNIVERSAL_IDENTIFIER,
} from './offering-line-package.field';

export default defineField({
  universalIdentifier:
    PURCHASED_PACKAGE_OFFERING_LINES_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'offeringLines',
  label: 'Offering lines',
  icon: 'IconListDetails',
  relationTargetObjectMetadataUniversalIdentifier:
    OFFERING_LINE_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    OFFERING_LINE_PACKAGE_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
