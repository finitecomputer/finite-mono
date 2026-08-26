import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
} from 'twenty-sdk/define';

import { OFFERING_LINE_UNIVERSAL_IDENTIFIER } from '../objects/offering-line.object';
import { PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER } from '../objects/purchased-package.object';

export const OFFERING_LINE_PACKAGE_FIELD_UNIVERSAL_IDENTIFIER =
  '0c0f1762-4d25-40a9-92c2-f879617de610';
export const PURCHASED_PACKAGE_OFFERING_LINES_FIELD_UNIVERSAL_IDENTIFIER =
  '9d7bc79d-79a2-4c67-8570-31c43af20e64';

export default defineField({
  universalIdentifier: OFFERING_LINE_PACKAGE_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: OFFERING_LINE_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'purchasedPackage',
  label: 'Purchased package',
  icon: 'IconPackage',
  relationTargetObjectMetadataUniversalIdentifier:
    PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    PURCHASED_PACKAGE_OFFERING_LINES_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.RESTRICT,
    joinColumnName: 'purchasedPackageId',
  },
});
