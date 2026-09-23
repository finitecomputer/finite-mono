"""Ephemeral native requester identity with an optional registry-issued Sites assertion."""

import contextvars
import re
import time

_CONTEXT = contextvars.ContextVar("finite_native_requester", default=None)


def normalize(value):
    if not isinstance(value, dict):
        return None
    user = value.get("userId")
    email = value.get("email")
    assertion = value.get("sitesAssertion")
    expires = value.get("expiresAt")
    if (
        not isinstance(user, str)
        or re.fullmatch(r"[0-9a-f]{64}", user) is None
        or type(expires) is not int
        or expires <= time.time()
    ):
        return None
    identity = {"userId": user, "expiresAt": expires}
    if email is None and assertion is None:
        return identity
    if (
        not isinstance(email, str)
        or "@" not in email
        or len(email) > 254
        or not isinstance(assertion, str)
        or re.fullmatch(r"[0-9a-f]{64}", assertion) is None
    ):
        return None
    return {**identity, "email": email, "sitesAssertion": assertion}


def current():
    return _CONTEXT.get()


def bind(value):
    from gateway.session_context import _SESSION_PLATFORM, _SESSION_USER_ID

    value = normalize(value)
    tokens = [(_CONTEXT, _CONTEXT.set(value))]
    if value is not None:
        tokens.extend(
            (
                (_SESSION_PLATFORM, _SESSION_PLATFORM.set("local")),
                (_SESSION_USER_ID, _SESSION_USER_ID.set(value["userId"])),
            )
        )
    return tokens


def reset(tokens):
    for variable, token in reversed(tokens):
        variable.reset(token)
