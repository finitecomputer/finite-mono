import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
} from 'twenty-sdk/define';

import { CHARGE_UNIVERSAL_IDENTIFIER } from '../objects/charge.object';
import { PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER } from '../objects/purchased-package.object';

export const CHARGE_PACKAGE_FIELD_UNIVERSAL_IDENTIFIER =
  'ed873937-86b6-42ac-a72e-374d73a6f310';
export const PURCHASED_PACKAGE_CHARGES_FIELD_UNIVERSAL_IDENTIFIER =
  '819dccde-e26b-4470-8623-825070d408e5';

export default defineField({
  universalIdentifier: CHARGE_PACKAGE_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: CHARGE_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'purchasedPackage',
  label: 'Purchased package',
  icon: 'IconPackage',
  relationTargetObjectMetadataUniversalIdentifier:
    PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    PURCHASED_PACKAGE_CHARGES_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.SET_NULL,
    joinColumnName: 'purchasedPackageId',
  },
});
