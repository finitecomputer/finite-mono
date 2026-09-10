import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
} from 'twenty-sdk/define';

import { COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-arrangement.object';
import { PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER } from '../objects/purchased-package.object';

export const PURCHASED_PACKAGE_ARRANGEMENT_FIELD_UNIVERSAL_IDENTIFIER =
  'd15dbe27-37e3-4b22-8f9d-ba21a0bc5f05';
export const COMMERCIAL_ARRANGEMENT_PACKAGES_FIELD_UNIVERSAL_IDENTIFIER =
  'cf8953aa-737b-4921-8180-dfbccde92e8e';

export default defineField({
  universalIdentifier:
    PURCHASED_PACKAGE_ARRANGEMENT_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'arrangement',
  label: 'Commercial arrangement',
  icon: 'IconFileDescription',
  relationTargetObjectMetadataUniversalIdentifier:
    COMMERCIAL_ARRANGEMENT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    COMMERCIAL_ARRANGEMENT_PACKAGES_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.RESTRICT,
    joinColumnName: 'arrangementId',
  },
});
