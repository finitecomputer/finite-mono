# Make Sharing Capabilities Independent Of Brain Kind

Status: accepted

Amended 2026-08-07 by ADR-0045: Personal Brain governance includes the live
Personal Brain Agent Set rather than one Personal Agent.

Brain Invitations, Folder Invitations, and Folder Mounts will work across both
Personal and Organization Brains in every source and destination combination.
Brain kind determines governance—Personal Brain ownership and Personal Brain
Agent Set authority versus Organization Brain roles—not which sharing capabilities are
available. This hard-cuts the older server and client assumptions that limited
Brain Invitations, Share Links, or mount destinations to Organization Brains,
while preserving the rule that accepting an invitation or mount never
transfers Personal Brain ownership or silently grants an Organization Brain
admin role.
