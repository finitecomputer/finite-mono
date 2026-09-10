import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
} from 'twenty-sdk/define';

import { CHARGE_UNIVERSAL_IDENTIFIER } from '../objects/charge.object';
import { INCOMING_PAYMENT_UNIVERSAL_IDENTIFIER } from '../objects/incoming-payment.object';

export const INCOMING_PAYMENT_CHARGE_FIELD_UNIVERSAL_IDENTIFIER =
  'cd5c7c0e-74af-4160-97fd-3611703dd5d1';
export const CHARGE_INCOMING_PAYMENTS_FIELD_UNIVERSAL_IDENTIFIER =
  'd127b468-7e7f-40b9-b14c-06e87566af8e';

export default defineField({
  universalIdentifier: INCOMING_PAYMENT_CHARGE_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: INCOMING_PAYMENT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'charge',
  label: 'Charge',
  description: 'Charge directly settled by this payment in the ordinary MVP path',
  icon: 'IconReceipt',
  relationTargetObjectMetadataUniversalIdentifier: CHARGE_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    CHARGE_INCOMING_PAYMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.SET_NULL,
    joinColumnName: 'chargeId',
  },
});
