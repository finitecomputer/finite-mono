import { defineField, FieldType, RelationType } from 'twenty-sdk/define';

import { CHARGE_UNIVERSAL_IDENTIFIER } from '../objects/charge.object';
import { INCOMING_PAYMENT_UNIVERSAL_IDENTIFIER } from '../objects/incoming-payment.object';
import {
  CHARGE_INCOMING_PAYMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  INCOMING_PAYMENT_CHARGE_FIELD_UNIVERSAL_IDENTIFIER,
} from './incoming-payment-charge.field';

export default defineField({
  universalIdentifier: CHARGE_INCOMING_PAYMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: CHARGE_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'incomingPayments',
  label: 'Incoming payments',
  icon: 'IconCashBanknote',
  relationTargetObjectMetadataUniversalIdentifier:
    INCOMING_PAYMENT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    INCOMING_PAYMENT_CHARGE_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
