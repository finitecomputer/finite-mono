import { defineField, FieldType, RelationType } from 'twenty-sdk/define';

import { CHARGE_UNIVERSAL_IDENTIFIER } from '../objects/charge.object';
import { COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-account.object';
import {
  CHARGE_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
  COMMERCIAL_ACCOUNT_CHARGES_FIELD_UNIVERSAL_IDENTIFIER,
} from './charge-account.field';

export default defineField({
  universalIdentifier: COMMERCIAL_ACCOUNT_CHARGES_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'charges',
  label: 'Charges',
  icon: 'IconReceipt',
  relationTargetObjectMetadataUniversalIdentifier: CHARGE_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    CHARGE_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
