# Remove Shared Folder Source State

Status: accepted

Any Folder may be the subject of a Folder Invitation or Mount Offer without
first being marked as a Shared Folder Source or converted to restricted access.
The Folder's native access mode continues to govern identities inside its own
Brain, while explicit Guest grants govern invited identities and Mount
Participants. This hard-cuts the `sharedFolderSource` state and its preparatory
command, removes an otherwise redundant setup step, and prevents sharing from
silently changing the source Brain's existing or future internal access policy.
