import {
  defineView,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
  ViewOpenRecordIn,
  ViewType,
  ViewVisibility,
} from 'twenty-sdk/define';

import {
  COMPANY_CURRENT_MRR_FIELD_UNIVERSAL_IDENTIFIER,
  COMPANY_RELATIONSHIP_SUMMARY_FIELD_UNIVERSAL_IDENTIFIER,
  COMPANY_ROLES_FIELD_UNIVERSAL_IDENTIFIER,
} from '../constants/company-field-identifiers';

export const ORGANIZATION_DIRECTORY_VIEW_UNIVERSAL_IDENTIFIER =
  '5fc60277-fb52-425d-9498-891667c0d954';

export default defineView({
  universalIdentifier: ORGANIZATION_DIRECTORY_VIEW_UNIVERSAL_IDENTIFIER,
  name: 'Organization directory',
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: ViewType.TABLE,
  icon: 'IconBuildingCommunity',
  position: 0,
  visibility: ViewVisibility.WORKSPACE,
  openRecordIn: ViewOpenRecordIn.RECORD_PAGE,
  fields: [
    {
      universalIdentifier: 'ce978d8f-11c4-458b-bdd0-a85fbb7a5a5a',
      fieldMetadataUniversalIdentifier:
        STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.fields.name
          .universalIdentifier,
      position: 0,
      isVisible: true,
    },
    {
      universalIdentifier: '6c8d3f6e-4804-479d-a1f1-1c1302c9fa35',
      fieldMetadataUniversalIdentifier:
        STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.fields.domainName
          .universalIdentifier,
      position: 1,
      isVisible: true,
    },
    {
      universalIdentifier: 'b197f987-4bb8-4a24-9bdd-0d7bdd265d30',
      fieldMetadataUniversalIdentifier:
        COMPANY_ROLES_FIELD_UNIVERSAL_IDENTIFIER,
      position: 2,
      isVisible: true,
    },
    {
      universalIdentifier: '04339bcc-a977-44ce-9894-00a01a45ea68',
      fieldMetadataUniversalIdentifier:
        COMPANY_RELATIONSHIP_SUMMARY_FIELD_UNIVERSAL_IDENTIFIER,
      position: 3,
      isVisible: true,
    },
    {
      universalIdentifier: 'e2f042fd-5dbc-440d-8c9e-9f362f7041c1',
      fieldMetadataUniversalIdentifier:
        COMPANY_CURRENT_MRR_FIELD_UNIVERSAL_IDENTIFIER,
      position: 4,
      isVisible: true,
    },
  ],
});
