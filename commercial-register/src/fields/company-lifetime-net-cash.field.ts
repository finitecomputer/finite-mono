import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_LIFETIME_NET_CASH_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier:
    COMPANY_LIFETIME_NET_CASH_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.CURRENCY,
  name: 'lifetimeNetCashUsd',
  label: 'Lifetime net cash (derived)',
  description: 'USD cash received less refunds and reversals',
  icon: 'IconCash',
  isNullable: true,
  defaultValue: null,
  isUIEditable: false,
});
