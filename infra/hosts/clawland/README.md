# clawland — legacy fleet host

`clawland-ovh` is `15.204.108.57`. It is not the production Chat host or a
production closure builder. Chat runs on [the app plane](../../README.md).

Retained configuration and archives require fresh read-only inventory before
use. Do not restart a historical Chat writer, repoint DNS, or restore an old
SQLite copy over current history. Use the [Chat deployment](../../runbooks/deploy-finitechat-server.md)
and [recovery](../../runbooks/hosted-web-chat-recovery.md) runbooks.
