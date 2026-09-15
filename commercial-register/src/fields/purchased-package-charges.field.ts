import { defineField, FieldType, RelationType } from 'twenty-sdk/define';

import { CHARGE_UNIVERSAL_IDENTIFIER } from '../objects/charge.object';
import { PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER } from '../objects/purchased-package.object';
import {
  CHARGE_PACKAGE_FIELD_UNIVERSAL_IDENTIFIER,
  PURCHASED_PACKAGE_CHARGES_FIELD_UNIVERSAL_IDENTIFIER,
} from './charge-purchased-package.field';

export default defineField({
  universalIdentifier: PURCHASED_PACKAGE_CHARGES_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: PURCHASED_PACKAGE_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'charges',
  label: 'Charges',
  icon: 'IconReceipt',
  relationTargetObjectMetadataUniversalIdentifier: CHARGE_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    CHARGE_PACKAGE_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
