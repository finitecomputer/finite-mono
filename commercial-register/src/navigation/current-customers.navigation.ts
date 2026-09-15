import {
  defineNavigationMenuItem,
  NavigationMenuItemType,
} from 'twenty-sdk/define';

import { CURRENT_CUSTOMERS_VIEW_UNIVERSAL_IDENTIFIER } from '../views/current-customers.view';

export default defineNavigationMenuItem({
  universalIdentifier: 'a7cff254-0349-45bb-8f62-d16d327781a2',
  type: NavigationMenuItemType.VIEW,
  name: 'Current customers',
  icon: 'IconUsersGroup',
  position: 1,
  viewUniversalIdentifier: CURRENT_CUSTOMERS_VIEW_UNIVERSAL_IDENTIFIER,
});
