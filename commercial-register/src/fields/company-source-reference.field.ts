import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_SOURCE_REFERENCE_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier: COMPANY_SOURCE_REFERENCE_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.TEXT,
  name: 'sourceReference',
  label: 'Source reference',
  description: 'Optional evidence pointer for material organization facts',
  icon: 'IconLink',
  isNullable: true,
  defaultValue: null,
});
