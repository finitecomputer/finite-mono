import { defineField, FieldType, RelationType } from 'twenty-sdk/define';

import { COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-arrangement.object';
import { PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER } from '../objects/purchased-package.object';
import {
  COMMERCIAL_ARRANGEMENT_PACKAGES_FIELD_UNIVERSAL_IDENTIFIER,
  PURCHASED_PACKAGE_ARRANGEMENT_FIELD_UNIVERSAL_IDENTIFIER,
} from './purchased-package-arrangement.field';

export default defineField({
  universalIdentifier:
    COMMERCIAL_ARRANGEMENT_PACKAGES_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'purchasedPackages',
  label: 'Purchased packages',
  icon: 'IconPackage',
  relationTargetObjectMetadataUniversalIdentifier:
    PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    PURCHASED_PACKAGE_ARRANGEMENT_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
