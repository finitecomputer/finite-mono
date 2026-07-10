# Keep Browser And Desktop Plaintext Ephemeral

Status: accepted

Outside an explicitly created Vault Working Tree, browser and desktop clients
must not retain readable Pages, paths, drafts, indexes, graph data, previews,
or history across sessions. Plaintext remains Ephemeral Client Plaintext;
restart and crash recovery may persist Encrypted Recovery State, while Session
Lock clears readable memory and derived indexes are rebuilt after unlock.
