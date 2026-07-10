# Deny Automatic Plaintext Egress In First-Party Clients

Status: accepted

FiniteBrain's protocol authorizes Member Identities cryptographically and does
not attempt to control plaintext after an authorized client decrypts it.
First-party Finite clients nevertheless deny automatic Plaintext Egress,
including background Git synchronization, remote embeddings, content-bearing
analytics or diagnostics, plugins, and unprompted external service requests;
encrypted FiniteBrain sync remains allowed, explicitly initiated exports are
controller actions, and third-party client behavior is outside FiniteBrain's
enforcement boundary.
