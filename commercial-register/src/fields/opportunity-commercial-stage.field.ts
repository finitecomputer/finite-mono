import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

export const OPPORTUNITY_COMMERCIAL_STAGE_FIELD_UNIVERSAL_IDENTIFIER =
  '3b0feccb-187a-46fa-905a-b07721ba0a95';

export default defineField({
  universalIdentifier:
    OPPORTUNITY_COMMERCIAL_STAGE_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.universalIdentifier,
  type: FieldType.SELECT,
  name: 'commercialStage',
  label: 'Commercial stage',
  description: 'Small Finite pipeline for prospective and follow-on work',
  icon: 'IconProgress',
  defaultValue: "'EXPLORING'",
  options: [
    {
      id: 'c011ff81-0203-4c9e-818e-f37068854ef2',
      value: 'EXPLORING',
      label: 'Exploring',
      position: 0,
      color: 'gray',
    },
    {
      id: 'd3b96664-9c18-486c-be13-1c1d6c2d1919',
      value: 'PROPOSAL_DRAFTED',
      label: 'Proposal drafted',
      position: 1,
      color: 'yellow',
    },
    {
      id: '4ccb9734-f3d8-41e9-97bd-43c8053e026c',
      value: 'PROPOSAL_SENT',
      label: 'Proposal sent',
      position: 2,
      color: 'orange',
    },
    {
      id: 'cc219606-31e7-4a51-a871-dc040f5eea5b',
      value: 'WON',
      label: 'Won',
      position: 3,
      color: 'green',
    },
    {
      id: '47bf3300-7566-40fc-bfc6-4e9b7a46d33e',
      value: 'LOST',
      label: 'Lost',
      position: 4,
      color: 'red',
    },
    {
      id: '70e6a27e-1e99-4f86-a620-6d932ee40be8',
      value: 'PAUSED',
      label: 'Paused',
      position: 5,
      color: 'blue',
    },
  ],
});
