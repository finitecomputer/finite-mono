import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_RELATIONSHIP_SUMMARY_REFRESHED_AT_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier:
    COMPANY_RELATIONSHIP_SUMMARY_REFRESHED_AT_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.DATE_TIME,
  name: 'relationshipSummaryRefreshedAt',
  label: 'Relationship summary refreshed at',
  description: 'When the read-only summary was last projected from FiniteBrain',
  icon: 'IconRefresh',
  isNullable: true,
  defaultValue: null,
  isUIEditable: false,
});
