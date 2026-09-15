import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_BRAIN_PAGE_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier: COMPANY_BRAIN_PAGE_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.LINKS,
  name: 'brainPage',
  label: 'Brain page',
  description: 'Canonical FiniteBrain relationship page',
  icon: 'IconBrain',
  isNullable: true,
  defaultValue: null,
});
