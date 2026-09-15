import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

export default defineField({
  universalIdentifier: '009d5813-b5d8-4813-9ac4-93178328ba74',
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.universalIdentifier,
  type: FieldType.BOOLEAN,
  name: 'reconciliationWarning',
  label: 'Reconciliation warning',
  description: 'A visible marker that an agent found incomplete or conflicting facts',
  icon: 'IconAlertCircle',
  defaultValue: false,
});
