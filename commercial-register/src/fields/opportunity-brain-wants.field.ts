import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

export default defineField({
  universalIdentifier: 'f83b55aa-877c-4876-888f-8d586e76d398',
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.universalIdentifier,
  type: FieldType.LINKS,
  name: 'brainWants',
  label: 'Brain wants',
  description: 'Canonical FiniteBrain note describing what this organization wants',
  icon: 'IconBrain',
  isNullable: true,
  defaultValue: null,
});
