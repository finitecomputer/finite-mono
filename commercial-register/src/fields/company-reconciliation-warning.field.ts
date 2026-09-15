import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_RECONCILIATION_WARNING_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier:
    COMPANY_RECONCILIATION_WARNING_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.BOOLEAN,
  name: 'reconciliationWarning',
  label: 'Reconciliation warning',
  description: 'Marks an incomplete or ambiguous organization fact',
  icon: 'IconAlertCircle',
  defaultValue: false,
});
