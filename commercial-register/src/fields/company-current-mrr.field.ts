import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_CURRENT_MRR_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier: COMPANY_CURRENT_MRR_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.CURRENCY,
  name: 'currentMrrUsd',
  label: 'Current MRR (derived)',
  description: 'USD MRR rebuilt from active recurring price terms',
  icon: 'IconRepeat',
  isNullable: true,
  defaultValue: null,
  isUIEditable: false,
});
