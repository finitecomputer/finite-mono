import { defineApplicationRole } from 'twenty-sdk/define';

import { DEFAULT_ROLE_UNIVERSAL_IDENTIFIER } from './constants/universal-identifiers';

export default defineApplicationRole({
  universalIdentifier: DEFAULT_ROLE_UNIVERSAL_IDENTIFIER,
  label: 'Finite Commercial Register application role',
  description: 'No autonomous Twenty capabilities in the MVP',
  canReadAllObjectRecords: false,
  canUpdateAllObjectRecords: false,
  canSoftDeleteAllObjectRecords: false,
  canDestroyAllObjectRecords: false,
  canBeAssignedToAgents: false,
  canBeAssignedToApiKeys: false,
  canBeAssignedToUsers: false,
});
