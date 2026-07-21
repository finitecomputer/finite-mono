# Make Brain Working Trees Explicit Persistent Plaintext

Status: accepted

A Brain Working Tree is an explicitly created durable plaintext projection,
not a disposable cache: readable files remain on the Trusted Device after
`fbrain` exits or the device restarts until the controller removes the Working
Tree.
Creation must disclose that boundary, removing Session Folder Keys does not
claim to erase or hide those files, and browser or desktop caches remain
ephemeral by default unless the controller separately chooses to create a Brain
Working Tree.
