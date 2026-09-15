import { defineField, FieldType, RelationType } from 'twenty-sdk/define';

import { COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-account.object';
import { INCOMING_PAYMENT_UNIVERSAL_IDENTIFIER } from '../objects/incoming-payment.object';
import {
  COMMERCIAL_ACCOUNT_INCOMING_PAYMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  INCOMING_PAYMENT_PAYER_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
} from './incoming-payment-payer-account.field';

export default defineField({
  universalIdentifier:
    COMMERCIAL_ACCOUNT_INCOMING_PAYMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'incomingPayments',
  label: 'Incoming payments',
  icon: 'IconCashBanknote',
  relationTargetObjectMetadataUniversalIdentifier:
    INCOMING_PAYMENT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    INCOMING_PAYMENT_PAYER_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.ONE_TO_MANY,
  },
});
