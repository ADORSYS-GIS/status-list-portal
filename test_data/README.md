# Test data

`admin-keys/` holds throwaway private keys that tests sign admin tokens with, one per algorithm the Status List Server accepts. They are public, so never list them in a registry file or use them anywhere else.

`status-list-server/issuers.yaml` is the one exception: it is the registry of a Status List Server started only for the `live_admin_api` tests, and lists the public half of `admin-keys/es256.pem` as its admin. Never use it for a server anyone else can reach.
