import {
  defineNavigationMenuItem,
  NavigationMenuItemType,
} from 'twenty-sdk/define';

import { OPEN_OPPORTUNITIES_VIEW_UNIVERSAL_IDENTIFIER } from '../views/open-opportunities.view';

export default defineNavigationMenuItem({
  universalIdentifier: 'ebb205f5-7eff-4416-aa92-86b4b9f0f012',
  type: NavigationMenuItemType.VIEW,
  name: 'Open opportunities',
  icon: 'IconTargetArrow',
  position: 2,
  viewUniversalIdentifier: OPEN_OPPORTUNITIES_VIEW_UNIVERSAL_IDENTIFIER,
});
