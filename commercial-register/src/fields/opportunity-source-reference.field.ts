import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

export default defineField({
  universalIdentifier: '248e4518-b18a-4ac1-8939-c87247781831',
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.universalIdentifier,
  type: FieldType.TEXT,
  name: 'sourceReference',
  label: 'Source reference',
  description: 'Meeting note, invoice, message, or other evidence for this record',
  icon: 'IconLink',
  isNullable: true,
  defaultValue: null,
});
