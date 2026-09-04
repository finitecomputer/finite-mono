import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_IS_CURRENT_CUSTOMER_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier:
    COMPANY_IS_CURRENT_CUSTOMER_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.BOOLEAN,
  name: 'isCurrentCustomer',
  label: 'Current customer (derived)',
  description: 'True when an owned account has an active offering line',
  icon: 'IconCircleCheck',
  defaultValue: false,
  isUIEditable: false,
});
