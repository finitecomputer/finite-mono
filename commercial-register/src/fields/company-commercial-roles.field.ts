import {
  defineField,
  FieldType,
  STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS,
} from 'twenty-sdk/define';

import { COMPANY_ROLES_FIELD_UNIVERSAL_IDENTIFIER } from '../constants/company-field-identifiers';

export default defineField({
  universalIdentifier: COMPANY_ROLES_FIELD_UNIVERSAL_IDENTIFIER,
  objectUniversalIdentifier:
    STANDARD_OBJECT_UNIVERSAL_IDENTIFIERS.company.universalIdentifier,
  type: FieldType.MULTI_SELECT,
  name: 'commercialRoles',
  label: 'Commercial roles',
  description: 'Current roles in Finite commercial relationships',
  icon: 'IconTags',
  options: [
    {
      id: 'd22992bf-1913-4b7e-9f99-5531faa80539',
      value: 'PROSPECT',
      label: 'Prospect',
      position: 0,
      color: 'blue',
    },
    {
      id: '92005a17-7619-4c1c-80df-9de9e3ed680c',
      value: 'CUSTOMER',
      label: 'Customer',
      position: 1,
      color: 'green',
    },
    {
      id: '11205bf2-8f17-49b6-b393-aea483d34296',
      value: 'SPONSOR',
      label: 'Sponsor',
      position: 2,
      color: 'purple',
    },
    {
      id: '52868f8a-ecd5-475a-9a7b-33d8baa5b372',
      value: 'PARTNER',
      label: 'Partner',
      position: 3,
      color: 'orange',
    },
    {
      id: '8ebcd248-bb7f-4aac-8b43-3fe53f30bdbf',
      value: 'FORMER_CUSTOMER',
      label: 'Former customer',
      position: 4,
      color: 'gray',
    },
  ],
});
