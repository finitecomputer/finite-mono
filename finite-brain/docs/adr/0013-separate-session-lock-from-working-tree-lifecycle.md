# Separate Session Lock From Working Tree Lifecycle

Status: accepted

FiniteBrain uses Session Lock only when a trusted client clears Session Folder
Keys and temporary plaintext, hides content, and blocks automatic reopening.
A persistent Brain Working Tree is instead Paused when `fbrain` stops sync,
signing, and key opening while leaving ordinary files readable, and Removal is
the separate explicit deletion action; this avoids promising confidentiality
that a plaintext Working Tree cannot provide.
