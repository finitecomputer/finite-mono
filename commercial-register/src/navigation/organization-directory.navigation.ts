import {
  defineNavigationMenuItem,
  NavigationMenuItemType,
} from 'twenty-sdk/define';

import { ORGANIZATION_DIRECTORY_VIEW_UNIVERSAL_IDENTIFIER } from '../views/organization-directory.view';

export default defineNavigationMenuItem({
  universalIdentifier: '88f2a19d-b327-47d1-9174-a76a8e87778a',
  type: NavigationMenuItemType.VIEW,
  name: 'Organizations',
  icon: 'IconBuildingCommunity',
  position: 0,
  viewUniversalIdentifier: ORGANIZATION_DIRECTORY_VIEW_UNIVERSAL_IDENTIFIER,
});
