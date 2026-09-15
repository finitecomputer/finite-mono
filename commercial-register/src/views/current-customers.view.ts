import {
  defineView,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
  ViewFilterOperand,
  ViewOpenRecordIn,
  ViewType,
  ViewVisibility,
} from 'twenty-sdk/define';

import {
  COMPANY_CURRENT_MRR_FIELD_UNIVERSAL_IDENTIFIER,
  COMPANY_IS_CURRENT_CUSTOMER_FIELD_UNIVERSAL_IDENTIFIER,
  COMPANY_LIFETIME_NET_CASH_FIELD_UNIVERSAL_IDENTIFIER,
  COMPANY_RELATIONSHIP_SUMMARY_FIELD_UNIVERSAL_IDENTIFIER,
  COMPANY_ROLES_FIELD_UNIVERSAL_IDENTIFIER,
} from '../constants/company-field-identifiers';

export const CURRENT_CUSTOMERS_VIEW_UNIVERSAL_IDENTIFIER =
  'a9a27c5e-c63f-46e7-87a8-51ee2e560bb0';

export default defineView({
  universalIdentifier: CURRENT_CUSTOMERS_VIEW_UNIVERSAL_IDENTIFIER,
  name: 'Current customers',
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: ViewType.TABLE,
  icon: 'IconUsersGroup',
  position: 1,
  visibility: ViewVisibility.WORKSPACE,
  openRecordIn: ViewOpenRecordIn.RECORD_PAGE,
  fields: [
    {
      universalIdentifier: '2ed46eb5-9fd8-46e6-87ac-c5c1fb286163',
      fieldMetadataUniversalIdentifier:
        STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.fields.name
          .universalIdentifier,
      position: 0,
      isVisible: true,
    },
    {
      universalIdentifier: '716836ed-7486-4a82-b66c-d11a375ac156',
      fieldMetadataUniversalIdentifier:
        COMPANY_ROLES_FIELD_UNIVERSAL_IDENTIFIER,
      position: 1,
      isVisible: true,
    },
    {
      universalIdentifier: 'fe71e349-cd09-4082-b70d-5d1c5b8da95a',
      fieldMetadataUniversalIdentifier:
        COMPANY_CURRENT_MRR_FIELD_UNIVERSAL_IDENTIFIER,
      position: 2,
      isVisible: true,
    },
    {
      universalIdentifier: '306893d6-3501-4976-a053-e4821207f481',
      fieldMetadataUniversalIdentifier:
        COMPANY_LIFETIME_NET_CASH_FIELD_UNIVERSAL_IDENTIFIER,
      position: 3,
      isVisible: true,
    },
    {
      universalIdentifier: 'f9e6caee-67b9-4313-93e6-9a0f9cdfdea5',
      fieldMetadataUniversalIdentifier:
        COMPANY_RELATIONSHIP_SUMMARY_FIELD_UNIVERSAL_IDENTIFIER,
      position: 4,
      isVisible: true,
    },
  ],
  filters: [
    {
      universalIdentifier: '13b42730-0433-4e8b-b41e-36b8890e29bf',
      fieldMetadataUniversalIdentifier:
        COMPANY_IS_CURRENT_CUSTOMER_FIELD_UNIVERSAL_IDENTIFIER,
      operand: ViewFilterOperand.IS,
      value: true,
    },
  ],
});
