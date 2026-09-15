import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_RELATIONSHIP_SUMMARY_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier:
    COMPANY_RELATIONSHIP_SUMMARY_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.TEXT,
  name: 'relationshipSummary',
  label: 'Relationship summary',
  description: 'Short refresh-labeled summary; narrative remains in Brain',
  icon: 'IconNotes',
  isNullable: true,
  defaultValue: null,
  isUIEditable: false,
});
