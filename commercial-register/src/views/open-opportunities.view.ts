import {
  defineView,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
  ViewFilterOperand,
  ViewOpenRecordIn,
  ViewType,
  ViewVisibility,
} from 'twenty-sdk/define';

import { OPPORTUNITY_COMMERCIAL_STAGE_FIELD_UNIVERSAL_IDENTIFIER } from '../fields/opportunity-commercial-stage.field';

export const OPEN_OPPORTUNITIES_VIEW_UNIVERSAL_IDENTIFIER =
  '2aa6a8ce-e129-4a3f-b339-c0d0aac097ea';

export default defineView({
  universalIdentifier: OPEN_OPPORTUNITIES_VIEW_UNIVERSAL_IDENTIFIER,
  name: 'Open opportunities',
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.universalIdentifier,
  type: ViewType.KANBAN,
  icon: 'IconTargetArrow',
  position: 2,
  visibility: ViewVisibility.WORKSPACE,
  openRecordIn: ViewOpenRecordIn.RECORD_PAGE,
  mainGroupByFieldMetadataUniversalIdentifier:
    OPPORTUNITY_COMMERCIAL_STAGE_FIELD_UNIVERSAL_IDENTIFIER,
  shouldHideEmptyGroups: false,
  fields: [
    {
      universalIdentifier: '876805e4-d319-4ba3-80a2-f73a8673dfcd',
      fieldMetadataUniversalIdentifier:
        STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.fields.name
          .universalIdentifier,
      position: 0,
      isVisible: true,
    },
    {
      universalIdentifier: '5354d684-3c34-4dc2-8b47-1eac19c96452',
      fieldMetadataUniversalIdentifier:
        STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.fields.company
          .universalIdentifier,
      position: 1,
      isVisible: true,
    },
    {
      universalIdentifier: '533fbb72-8ff5-40ed-a279-172dc80f7100',
      fieldMetadataUniversalIdentifier:
        STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.opportunity.fields.amount
          .universalIdentifier,
      position: 2,
      isVisible: true,
    },
    {
      universalIdentifier: 'ed40596e-ed8f-4ddd-a644-3fb90341014c',
      fieldMetadataUniversalIdentifier:
        OPPORTUNITY_COMMERCIAL_STAGE_FIELD_UNIVERSAL_IDENTIFIER,
      position: 3,
      isVisible: true,
    },
  ],
  filters: [
    {
      universalIdentifier: '60ba7c40-ae4c-485c-bd0d-3fb982a1add5',
      fieldMetadataUniversalIdentifier:
        OPPORTUNITY_COMMERCIAL_STAGE_FIELD_UNIVERSAL_IDENTIFIER,
      operand: ViewFilterOperand.IS_NOT,
      value: ['WON'],
      positionInViewFilterGroup: 0,
    },
    {
      universalIdentifier: 'b9f8cef5-fc29-4e68-8dcf-de5dc73c8fc7',
      fieldMetadataUniversalIdentifier:
        OPPORTUNITY_COMMERCIAL_STAGE_FIELD_UNIVERSAL_IDENTIFIER,
      operand: ViewFilterOperand.IS_NOT,
      value: ['LOST'],
      positionInViewFilterGroup: 1,
    },
  ],
  groups: [
    {
      universalIdentifier: '78fb9cb5-6254-440e-94a0-f957434231d1',
      fieldValue: 'EXPLORING',
      position: 0,
      isVisible: true,
    },
    {
      universalIdentifier: '1dd29126-caff-46ec-bfb5-8394e1a4260b',
      fieldValue: 'PROPOSAL_DRAFTED',
      position: 1,
      isVisible: true,
    },
    {
      universalIdentifier: '94a03396-e2f7-43b7-bcdd-c229dbfd6ff0',
      fieldValue: 'PROPOSAL_SENT',
      position: 2,
      isVisible: true,
    },
    {
      universalIdentifier: 'b90964ca-6196-4b4a-9490-c5bc81222c4d',
      fieldValue: 'PAUSED',
      position: 3,
      isVisible: true,
    },
    {
      universalIdentifier: 'e4106c2f-29d3-4c31-bcc4-28af80c8620f',
      fieldValue: 'WON',
      position: 4,
      isVisible: false,
    },
    {
      universalIdentifier: '693c5f1e-6d17-4ad2-9983-072198cc03cc',
      fieldValue: 'LOST',
      position: 5,
      isVisible: false,
    },
  ],
});
