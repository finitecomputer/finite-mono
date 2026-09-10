import {
  defineField,
  FieldType,
  OnDeleteAction,
  RelationType,
} from 'twenty-sdk/define';

import { COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER } from '../objects/commercial-account.object';
import { INCOMING_PAYMENT_UNIVERSAL_IDENTIFIER } from '../objects/incoming-payment.object';

export const INCOMING_PAYMENT_PAYER_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER =
  'f4776a4b-3e09-49e9-9ba9-faad377751f0';
export const COMMERCIAL_ACCOUNT_INCOMING_PAYMENTS_FIELD_UNIVERSAL_IDENTIFIER =
  '8faaabee-b31a-4a97-ae0d-cff8fb904efe';

export default defineField({
  universalIdentifier:
    INCOMING_PAYMENT_PAYER_ACCOUNT_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier: INCOMING_PAYMENT_UNIVERSAL_IDENTIFIER,
  type: FieldType.RELATION,
  name: 'payerAccount',
  label: 'Payer account',
  description: 'Account from which Finite actually received value',
  icon: 'IconBuildingBank',
  relationTargetObjectMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_UNIVERSAL_IDENTIFIER,
  relationTargetFieldMetadataUniversalIdentifier:
    COMMERCIAL_ACCOUNT_INCOMING_PAYMENTS_FIELD_UNIVERSAL_IDENTIFIER,
  universalSettings: {
    relationType: RelationType.MANY_TO_ONE,
    onDelete: OnDeleteAction.RESTRICT,
    joinColumnName: 'payerAccountId',
  },
});
